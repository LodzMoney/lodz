//! `Stope` -- a risk-profiled vault, at `["stope", stope_id(u8)]`.
//!
//! Three stopes exist and no more: 0 conservative, 1 balanced, 2 aggressive.
//! The profile is not a label. It is the ceiling on how much of the stope's
//! allocation may sit on emissions-backed seams, how much may sit on
//! counterparty-funded seams, and how high a seam's headlamp tier may be. All
//! three are enforced by `register_seam` and `update_seam_allocation`, so the
//! difference between the profiles is a set of transactions the chain rejects
//! rather than a description on a page.

use anchor_lang::prelude::*;

use crate::errors::LodzError;
use crate::math::index_delta;
use crate::state::{RiskProfile, YieldKind};

/// One risk-profiled vault: its shares, its principal, and three separate
/// yield accumulators.
///
/// There is no combined `total_yield` field anywhere in this struct, and that
/// is deliberate. Sustainable, emissions and counterparty yield are
/// accumulated, indexed and reported apart from each other all the way down to
/// the individual miner, because a single blended number cannot answer the
/// only questions that matter about a BTC yield product: how much of this
/// survives when the emission schedule ends, and how much of it is somebody
/// else's loss rather than a fee anyone is paying.
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

    // -- appended 2026-08-16, into the reserved tail --------------------------
    //
    // These sit after every field that existed before them and take their
    // bytes out of `reserved`, so `LEN` is unchanged and an account written by
    // the previous deployment decodes with both of them zero -- which is the
    // correct starting value. Reordering anything above this line would
    // instead reinterpret live bytes.
    /// Per-share accumulator for counterparty yield, scaled by
    /// `YIELD_INDEX_SCALE`.
    pub yield_index_counterparty: u128,
    /// Lifetime realized counterparty yield, in internal accounting units.
    pub realized_counterparty: u64,
    /// The part of `allocated_bps` sitting on seams whose yield kind is
    /// [`YieldKind::Counterparty`]. Capped by
    /// `RiskProfile::max_counterparty_bps`.
    ///
    /// Unlike the two accumulators above, this one is a *running total over
    /// existing seams*, so a stope that already held counterparty weight when
    /// this field was introduced reads 0 here and under-counts. On a stope
    /// whose ceiling is 0 that is harmless -- every further change is rejected
    /// anyway -- but it is the reason a field like this cannot simply be
    /// appended on a live deployment without reconciling it.
    pub counterparty_bps: u16,

    pub _padding: [u8; 2],
    pub reserved: [u8; 38],
}

impl Stope {
    pub const LEN: usize = 1 * 4 // stope_id, risk_profile, paused, bump
        + 2 * 3                  // allocated_bps, emissions_bps, seam_count
        + 4                      // miner_count
        + 16 * 2                 // yield_index_sustainable, yield_index_emissions
        + 8 * 6                  // total_shares, total_deposits, pending_redemption,
                                 // total_redeemed, realized_sustainable, realized_emissions
        + 8 * 3                  // created_at, last_rebalance_at, last_accrual_at
        + 16                     // yield_index_counterparty
        + 8                      // realized_counterparty
        + 2                      // counterparty_bps
        + 2                      // _padding
        + 38; // reserved (was 64; 26 bytes went to the three fields above)

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
            YieldKind::Counterparty => {
                self.realized_counterparty = self
                    .realized_counterparty
                    .checked_add(amount)
                    .ok_or(LodzError::MathOverflow)?;
                self.yield_index_counterparty = self
                    .yield_index_counterparty
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

        // Each kind moves its own counter and no other. Folding counterparty
        // weight into `emissions_bps` would report trader losses as scheduled
        // yield, which is the confusion this program exists to prevent, so the
        // two ceilings are tracked and checked separately.
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

        let counterparty = if kind == YieldKind::Counterparty {
            self.counterparty_bps
                .saturating_sub(previous_bps)
                .checked_add(next_bps)
                .ok_or(LodzError::MathOverflow)?
        } else {
            self.counterparty_bps
        };
        require!(
            counterparty <= self.risk_profile.max_counterparty_bps(),
            LodzError::CounterpartyAllocationExceeded
        );

        self.allocated_bps = allocated;
        self.emissions_bps = emissions;
        self.counterparty_bps = counterparty;
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

    /// Three accumulators, three indices, and no crosstalk between any pair.
    #[test]
    fn the_three_kinds_never_touch_each_others_accumulators() {
        let mut s = stope(RiskProfile::Aggressive);
        s.total_shares = 100_000_000;

        s.accrue(YieldKind::Sustainable, 1_000_000, 10).unwrap();
        s.accrue(YieldKind::Emissions, 2_000_000, 20).unwrap();
        s.accrue(YieldKind::Counterparty, 4_000_000, 30).unwrap();

        assert_eq!(s.realized_sustainable, 1_000_000);
        assert_eq!(s.realized_emissions, 2_000_000);
        assert_eq!(s.realized_counterparty, 4_000_000);
        assert_eq!(s.yield_index_sustainable, YIELD_INDEX_SCALE / 100);
        assert_eq!(s.yield_index_emissions, YIELD_INDEX_SCALE / 50);
        assert_eq!(s.yield_index_counterparty, YIELD_INDEX_SCALE / 25);

        // A counterparty accrual moves neither of the other two, in either
        // direction. Filing trader losses under "sustainable" is the specific
        // lie this enum exists to make impossible.
        let (sus, emi) = (s.yield_index_sustainable, s.yield_index_emissions);
        s.accrue(YieldKind::Counterparty, 1_000_000, 40).unwrap();
        assert_eq!(s.yield_index_sustainable, sus);
        assert_eq!(s.yield_index_emissions, emi);
        assert_eq!(s.realized_sustainable, 1_000_000);
        assert_eq!(s.realized_emissions, 2_000_000);
        assert_eq!(s.realized_counterparty, 5_000_000);
    }

    /// Counterparty weight is tracked apart from emissions weight.
    ///
    /// It must not consume the emissions ceiling, or a stope holding trader
    /// losses would report itself as holding scheduled emissions.
    #[test]
    fn counterparty_allocation_does_not_consume_the_emissions_ceiling() {
        let mut s = stope(RiskProfile::Aggressive);

        s.reallocate(YieldKind::Counterparty, 0, 3_000).unwrap();
        assert_eq!(s.allocated_bps, 3_000);
        assert_eq!(s.counterparty_bps, 3_000);
        assert_eq!(s.emissions_bps, 0, "counterparty is not emissions");

        // The full emissions ceiling is still available.
        s.reallocate(YieldKind::Emissions, 0, 6_000).unwrap();
        assert_eq!(s.emissions_bps, 6_000);
        assert_eq!(s.allocated_bps, 9_000);
        assert_eq!(s.counterparty_bps, 3_000, "and untouched by the emissions change");
    }

    /// The two profiles that publish "no counterparty" reject it outright.
    ///
    /// apps/web CHAMBER_POLICY carries admitsCounterparty=false for both
    /// conservative and balanced, and the conservative stance reads "nothing
    /// funded by somebody else's loss". Before this ceiling existed the chain
    /// accepted counterparty weight on either of them -- measured on devnet,
    /// where a 2000 bps counterparty seam registered against the balanced
    /// stope without complaint.
    #[test]
    fn profiles_that_publish_no_counterparty_reject_it_on_chain() {
        for profile in [RiskProfile::Conservative, RiskProfile::Balanced] {
            let mut s = stope(profile);
            assert_eq!(profile.max_counterparty_bps(), 0);
            assert!(
                s.reallocate(YieldKind::Counterparty, 0, 1).is_err(),
                "{profile:?} must refuse even one basis point"
            );
            assert_eq!(s.counterparty_bps, 0, "a rejected change must not apply");
            assert_eq!(s.allocated_bps, 0);

            // The other kinds are unaffected.
            s.reallocate(YieldKind::Sustainable, 0, 5_000).unwrap();
            assert_eq!(s.allocated_bps, 5_000);
        }
    }

    #[test]
    fn the_forward_profile_caps_counterparty_rather_than_banning_it() {
        let mut s = stope(RiskProfile::Aggressive);
        assert_eq!(RiskProfile::Aggressive.max_counterparty_bps(), 3_000);

        s.reallocate(YieldKind::Counterparty, 0, 3_000).unwrap();
        // One basis point over the published cap is refused.
        assert!(s.reallocate(YieldKind::Counterparty, 0, 1).is_err());
        assert_eq!(s.counterparty_bps, 3_000);

        // Winding an existing counterparty seam down is always allowed.
        s.reallocate(YieldKind::Counterparty, 3_000, 0).unwrap();
        assert_eq!(s.counterparty_bps, 0);
    }

    #[test]
    fn accrual_into_an_empty_stope_is_rejected() {
        let mut s = stope(RiskProfile::Balanced);
        assert!(s.accrue(YieldKind::Sustainable, 1_000, 1).is_err());
        assert_eq!(s.realized_sustainable, 0);
    }
}
