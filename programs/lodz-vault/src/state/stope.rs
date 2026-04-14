//! `Stope` -- a risk-profiled vault, at `["stope", stope_id(u8)]`.
//!
//! Three stopes exist and no more: 0 conservative, 1 balanced, 2 aggressive.
//! The profile is not a label. It is the ceiling on how much of the stope's
//! allocation may sit on emissions-backed seams and on how high a seam's
//! headlamp tier may be, and both ceilings are enforced by
//! `register_seam` and `update_seam_allocation`.

use anchor_lang::prelude::*;

use crate::errors::LodzError;
use crate::math::index_delta;
use crate::state::{RiskProfile, YieldKind};

/// One risk-profiled vault: its shares, its principal, and two separate yield
/// accumulators.
///
/// There is no combined `total_yield` field anywhere in this struct, and that
/// is deliberate. Sustainable and emissions yield are accumulated, indexed and
/// reported apart from each other all the way down to the individual miner,
/// because a single blended number cannot answer the only question that
/// matters about a BTC yield product: how much of this survives when the
/// emission schedule ends.
#[account]
pub struct Stope {
    pub stope_id: u8,
    /// Fixed to `stope_id` by `RiskProfile::from_id`.
    pub risk_profile: RiskProfile,
    pub paused: bool,
    pub bump: u8,

    /// Sum of `allocation_bps` over this stope's active seams. Never above
    /// 10_000.
    pub allocated_bps: u16,
    /// The part of `allocated_bps` that sits on seams whose yield kind is
    /// [`YieldKind::Emissions`]. Capped by
    /// `RiskProfile::max_emissions_bps`.
    ///
    /// This is the headline honesty number for the stope, maintained
    /// incrementally on every allocation change so a reader never has to sum
    /// the seam accounts to learn it.
    pub emissions_bps: u16,
    pub seam_count: u16,
    pub miner_count: u32,

    /// Per-share accumulator for sustainable yield, scaled by
    /// `YIELD_INDEX_SCALE`.
    pub yield_index_sustainable: u128,
    /// Per-share accumulator for emissions yield, scaled by
    /// `YIELD_INDEX_SCALE`.
    pub yield_index_emissions: u128,

    /// Outstanding principal claims. A share is one internal accounting unit
    /// of principal.
    pub total_shares: u64,
    /// Principal actually held against those claims. Equal to `total_shares`
    /// while no loss has been recorded; the redemption path divides by
    /// `total_shares` rather than assuming parity, so a future loss-recording
    /// instruction does not require re-deriving every payout formula.
    pub total_deposits: u64,
    /// Internal units reserved by queued Orecart tickets and not yet paid out.
    pub pending_redemption: u64,
    /// Internal units paid out over this stope's lifetime.
    pub total_redeemed: u64,

    /// Lifetime realized sustainable yield, in internal accounting units.
    pub realized_sustainable: u64,
    /// Lifetime realized emissions yield, in internal accounting units.
    pub realized_emissions: u64,

    pub created_at: i64,
    pub last_rebalance_at: i64,
    pub last_accrual_at: i64,

    pub _padding: [u8; 2],
    pub reserved: [u8; 64],
}

impl Stope {
    pub const LEN: usize = 1 * 4 // stope_id, risk_profile, paused, bump
        + 2 * 3                  // allocated_bps, emissions_bps, seam_count
        + 4                      // miner_count
        + 16 * 2                 // yield_index_sustainable, yield_index_emissions
        + 8 * 6                  // total_shares, total_deposits, pending_redemption,
                                 // total_redeemed, realized_sustainable, realized_emissions
        + 8 * 3                  // created_at, last_rebalance_at, last_accrual_at
        + 2                      // _padding
        + 64; // reserved

    /// Record realized yield of one kind, moving both the lifetime total and
    /// the per-share index for that kind and nothing else.
    ///
    /// Errors when the stope holds no shares rather than dropping the accrual,
    /// so `realized_*` can never report yield that no miner is able to claim.
    pub fn accrue(&mut self, kind: YieldKind, amount: u64, now: i64) -> Result<u128> {
        let delta = index_delta(amount, self.total_shares)?;

        match kind {
            YieldKind::Sustainable => {
                self.realized_sustainable = self
                    .realized_sustainable
                    .checked_add(amount)
                    .ok_or(LodzError::MathOverflow)?;
                self.yield_index_sustainable = self
                    .yield_index_sustainable
                    .checked_add(delta)
                    .ok_or(LodzError::MathOverflow)?;
            }
            YieldKind::Emissions => {
                self.realized_emissions = self
                    .realized_emissions
                    .checked_add(amount)
                    .ok_or(LodzError::MathOverflow)?;
                self.yield_index_emissions = self
                    .yield_index_emissions
                    .checked_add(delta)
                    .ok_or(LodzError::MathOverflow)?;
            }
        }

        self.last_accrual_at = now;
        Ok(delta)
    }

    /// Apply an allocation change for one seam, keeping `allocated_bps` and
    /// `emissions_bps` in step and refusing anything the risk profile forbids.
    pub fn reallocate(
        &mut self,
        kind: YieldKind,
        previous_bps: u16,
        next_bps: u16,
    ) -> Result<()> {
        require!(
            next_bps <= crate::math::MAX_BPS,
            LodzError::InvalidAllocationBps
        );

        let allocated = self
            .allocated_bps
            .saturating_sub(previous_bps)
            .checked_add(next_bps)
            .ok_or(LodzError::MathOverflow)?;
        require!(
            allocated <= crate::math::MAX_BPS,
            LodzError::AllocationExceeded
        );

        let emissions = if kind == YieldKind::Emissions {
            self.emissions_bps
                .saturating_sub(previous_bps)
                .checked_add(next_bps)
                .ok_or(LodzError::MathOverflow)?
        } else {
            self.emissions_bps
        };
        require!(
            emissions <= self.risk_profile.max_emissions_bps(),
            LodzError::EmissionsAllocationExceeded
        );

        self.allocated_bps = allocated;
        self.emissions_bps = emissions;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::YIELD_INDEX_SCALE;

    fn stope(profile: RiskProfile) -> Stope {
        let mut s = Stope::try_from_slice(&vec![0u8; Stope::LEN]).expect("decode");
        s.risk_profile = profile;
        s.stope_id = profile.id();
        s
    }

    #[test]
    fn accrual_keeps_the_two_kinds_apart() {
        let mut s = stope(RiskProfile::Balanced);
        s.total_shares = 100_000_000;

        s.accrue(YieldKind::Sustainable, 1_000_000, 100).unwrap();
        s.accrue(YieldKind::Emissions, 4_000_000, 200).unwrap();

        assert_eq!(s.realized_sustainable, 1_000_000);
        assert_eq!(s.realized_emissions, 4_000_000);
        assert_eq!(s.yield_index_sustainable, YIELD_INDEX_SCALE / 100);
        assert_eq!(s.yield_index_emissions, YIELD_INDEX_SCALE / 25);
        assert_eq!(s.last_accrual_at, 200);

        // A sustainable accrual must never move the emissions index, and the
        // reverse. Blending them is the failure this program exists to avoid.
        let before = s.yield_index_emissions;
        s.accrue(YieldKind::Sustainable, 1_000_000, 300).unwrap();
        assert_eq!(s.yield_index_emissions, before);
        assert_eq!(s.realized_emissions, 4_000_000);
    }

    #[test]
    fn a_conservative_stope_refuses_a_heavy_emissions_allocation() {
        let mut s = stope(RiskProfile::Conservative);

        // 20 % of the allocation on emissions is the ceiling ...
        s.reallocate(YieldKind::Emissions, 0, 2_000).unwrap();
        assert_eq!(s.emissions_bps, 2_000);
        assert_eq!(s.allocated_bps, 2_000);

        // ... and 20 % + 1 bp on a second emissions seam is rejected.
        assert!(s.reallocate(YieldKind::Emissions, 0, 1).is_err());
        assert_eq!(s.emissions_bps, 2_000, "a rejected change must not apply");

        // Sustainable seams can still fill the rest.
        s.reallocate(YieldKind::Sustainable, 0, 8_000).unwrap();
        assert_eq!(s.allocated_bps, 10_000);
        assert_eq!(s.emissions_bps, 2_000);
    }

    #[test]
    fn total_allocation_cannot_exceed_one_hundred_percent() {
        let mut s = stope(RiskProfile::Aggressive);
        s.reallocate(YieldKind::Sustainable, 0, 9_000).unwrap();
        assert!(s.reallocate(YieldKind::Sustainable, 0, 1_001).is_err());
        s.reallocate(YieldKind::Sustainable, 0, 1_000).unwrap();
        assert_eq!(s.allocated_bps, 10_000);
    }

    #[test]
    fn lowering_a_seam_allocation_frees_room_for_another() {
        let mut s = stope(RiskProfile::Balanced);
        s.reallocate(YieldKind::Emissions, 0, 5_000).unwrap();
        // Same seam moving 5000 -> 1000 releases 4000 bps of emissions room.
        s.reallocate(YieldKind::Emissions, 5_000, 1_000).unwrap();
        assert_eq!(s.emissions_bps, 1_000);
        assert_eq!(s.allocated_bps, 1_000);
        s.reallocate(YieldKind::Emissions, 0, 4_000).unwrap();
        assert_eq!(s.emissions_bps, 5_000);
    }

    #[test]
    fn accrual_into_an_empty_stope_is_rejected() {
        let mut s = stope(RiskProfile::Balanced);
        assert!(s.accrue(YieldKind::Sustainable, 1_000, 1).is_err());
        assert_eq!(s.realized_sustainable, 0);
    }
}
