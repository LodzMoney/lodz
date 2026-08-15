//! `Seam` -- one yield source, at `["seam", seam_id(u16 LE)]`.
//!
//! A seam is exactly one (venue, asset, yield kind) triple. Because the kind
//! is a property of the seam rather than of an accrual, `realized_yield` on
//! this account is unambiguously yield of that one kind, and the per-source
//! breakdown the product promises falls out of the account layout instead of
//! having to be reconstructed by an indexer.

use anchor_lang::prelude::*;

use crate::errors::LodzError;
use crate::state::YieldKind;

/// A registered yield source and its allocation within one stope.
#[account]
pub struct Seam {
    pub seam_id: u16,
    /// Share of the owning stope's capital routed here, in basis points.
    pub allocation_bps: u16,

    /// Stope this seam belongs to. A seam serves exactly one stope, so a
    /// stope's emissions exposure is the sum over its own seams and nothing
    /// else.
    pub stope_id: u8,
    /// Sustainable, emissions or counterparty. Immutable after registration:
    /// changing it would silently rewrite the meaning of `realized_yield`
    /// already booked under the old kind.
    pub yield_kind: YieldKind,
    /// Layer severity, 1 (lowest) to 5 (highest). Bounded by the owning
    /// stope's `RiskProfile::max_risk_tier`.
    ///
    /// See `Adit::risk_tier` for what the number is on, and
    /// `docs/risk-spec.md` 2.5 for what it does not yet have: there is no
    /// production custody ledger, so this value is asserted by whoever
    /// registers the seam rather than measured.
    pub risk_tier: u8,
    /// Inactive seams cannot accrue and cannot hold an allocation.
    pub active: bool,
    pub bump: u8,

    /// NUL-padded ASCII venue name, e.g. `kyros-lend`.
    pub venue: [u8; 32],
    /// On-chain program of the venue, when it has one. `Pubkey::default()`
    /// for a venue that is not a Solana program.
    pub venue_program: Pubkey,
    /// Mint of the asset deployed into this seam.
    pub asset_mint: Pubkey,
    /// For an emissions seam, the mint the emission is paid in. Required to be
    /// set for `YieldKind::Emissions` and required to be
    /// `Pubkey::default()` for `YieldKind::Sustainable`.
    pub emission_mint: Pubkey,

    /// Lifetime realized yield from this seam, in internal accounting units.
    /// One kind only, because the seam is one kind only.
    pub realized_yield: u64,
    pub accrual_count: u64,

    /// Unix timestamp at which this seam's emission schedule ends.
    ///
    /// Required to be non-zero and in the future for an emissions seam, and
    /// required to be zero for a sustainable one. `accrue_yield` refuses an
    /// emissions accrual once the chain clock has passed it, so a seam cannot
    /// keep reporting yield from a schedule that has run out.
    pub emission_ends_at: i64,
    pub registered_at: i64,
    pub last_accrual_at: i64,
    pub last_rebalance_at: i64,

    pub _padding: [u8; 7],
    pub reserved: [u8; 64],
}

impl Seam {
    pub const LEN: usize = 2 * 2 // seam_id, allocation_bps
        + 1 * 5                  // stope_id, yield_kind, risk_tier, active, bump
        + 32                     // venue
        + 32 * 3                 // venue_program, asset_mint, emission_mint
        + 8 * 2                  // realized_yield, accrual_count
        + 8 * 4                  // emission_ends_at, registered_at, last_accrual_at, last_rebalance_at
        + 7                      // _padding
        + 64; // reserved

    /// Enforce the invariant that ties a yield kind to its disclosure.
    ///
    /// An emissions seam that does not say when its emission ends, or that
    /// says it already ended, is exactly the shape of disclosure this protocol
    /// refuses to accept. A sustainable seam carrying emission fields is
    /// rejected too, so the kinds cannot be blurred by leaving stale values
    /// behind.
    ///
    /// A counterparty seam is held to the sustainable rule rather than the
    /// emissions one. Its yield has no schedule and therefore no honest end
    /// date, so letting it carry `emission_ends_at` would publish a promise
    /// about when it stops that nobody can keep. What bounds it instead is
    /// `RiskProfile::max_risk_tier`, checked by `register_seam`.
    pub fn validate_emission_fields(
        kind: YieldKind,
        emission_ends_at: i64,
        emission_mint: &Pubkey,
        now: i64,
    ) -> Result<()> {
        match kind {
            YieldKind::Sustainable | YieldKind::Counterparty => {
                require!(
                    emission_ends_at == 0 && *emission_mint == Pubkey::default(),
                    LodzError::EmissionFieldsOnSustainableSeam
                );
            }
            YieldKind::Emissions => {
                require!(emission_ends_at != 0, LodzError::EmissionEndMissing);
                require!(emission_ends_at > now, LodzError::EmissionEndInPast);
                require!(
                    *emission_mint != Pubkey::default(),
                    LodzError::EmissionMintMissing
                );
            }
        }
        Ok(())
    }

    /// Whether this seam may still book yield at `now`.
    ///
    /// Only an emissions seam has a closing date to enforce. A counterparty
    /// seam stops paying when the losing flow stops, which is not a fact the
    /// chain can know in advance -- so nothing here pretends to.
    pub fn accrual_window_open(&self, now: i64) -> bool {
        match self.yield_kind {
            YieldKind::Sustainable | YieldKind::Counterparty => true,
            YieldKind::Emissions => now < self.emission_ends_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn some_mint() -> Pubkey {
        Pubkey::new_from_array([7u8; 32])
    }

    #[test]
    fn an_emissions_seam_must_declare_a_future_end_and_a_mint() {
        // No end date.
        assert!(
            Seam::validate_emission_fields(YieldKind::Emissions, 0, &some_mint(), NOW).is_err()
        );
        // End date already passed.
        assert!(
            Seam::validate_emission_fields(YieldKind::Emissions, NOW - 1, &some_mint(), NOW)
                .is_err()
        );
        // End date, but no mint to name what is being emitted.
        assert!(Seam::validate_emission_fields(
            YieldKind::Emissions,
            NOW + 1,
            &Pubkey::default(),
            NOW
        )
        .is_err());
        // Complete disclosure.
        Seam::validate_emission_fields(YieldKind::Emissions, NOW + 86_400, &some_mint(), NOW)
            .unwrap();
    }

    #[test]
    fn a_sustainable_seam_must_leave_the_emission_fields_empty() {
        Seam::validate_emission_fields(YieldKind::Sustainable, 0, &Pubkey::default(), NOW).unwrap();
        assert!(
            Seam::validate_emission_fields(YieldKind::Sustainable, NOW + 1, &Pubkey::default(), NOW)
                .is_err()
        );
        assert!(
            Seam::validate_emission_fields(YieldKind::Sustainable, 0, &some_mint(), NOW).is_err()
        );
    }

    #[test]
    fn the_accrual_window_closes_when_the_emission_ends() {
        let mut seam = Seam::try_from_slice(&vec![0u8; Seam::LEN]).expect("decode");

        seam.yield_kind = YieldKind::Sustainable;
        assert!(seam.accrual_window_open(NOW));
        assert!(seam.accrual_window_open(i64::MAX));

        seam.yield_kind = YieldKind::Emissions;
        seam.emission_ends_at = NOW + 10;
        assert!(seam.accrual_window_open(NOW));
        assert!(seam.accrual_window_open(NOW + 9));
        assert!(!seam.accrual_window_open(NOW + 10));
        assert!(!seam.accrual_window_open(NOW + 11));
    }
}
