//! `Miner` -- one depositor's position in one stope, at
//! `["miner", owner, stope_id(u8)]`.

use anchor_lang::prelude::*;

use crate::errors::LodzError;
use crate::math::pending_from_index;
use crate::state::Stope;

/// A depositor's principal claim and their two separate yield balances.
///
/// The per-source split runs all the way down to here. A miner can read how
/// much of their own accrued yield came from something a counterparty is
/// paying versus from an emission schedule with an end date, which is the
/// number that decides whether the position is worth keeping.
#[account]
pub struct Miner {
    pub owner: Pubkey,
    pub stope_id: u8,
    pub bump: u8,
    /// Next Orecart ticket index for this position. Ticket PDAs are
    /// `["orecart", owner, ticket_index(u32 LE)]`, so this is the counter the
    /// client must read before building a redemption transaction.
    pub ticket_count: u32,

    /// Principal claim on the stope, in shares. One share is one internal
    /// accounting unit of principal at the time it was deposited.
    pub shares: u64,
    /// Lifetime internal units deposited.
    pub deposited: u64,
    /// Lifetime internal units paid out through claimed tickets.
    pub withdrawn: u64,
    /// Internal units sitting in this miner's queued, unclaimed tickets.
    ///
    /// Shares are burned at request time, not at claim time, so this is not a
    /// reservation against `shares` -- it is principal that has already left
    /// the share pool and is waiting out its delay. A miner in the queue stops
    /// earning yield the moment they queue, which is the honest reading of a
    /// redemption request.
    pub pending_redemption: u64,

    /// Settled sustainable yield, in internal accounting units.
    pub accrued_sustainable: u64,
    /// Settled emissions yield, in internal accounting units.
    pub accrued_emissions: u64,
    pub claimed_sustainable: u64,
    pub claimed_emissions: u64,

    /// Stope index values at the last settlement of this position.
    pub index_snapshot_sustainable: u128,
    pub index_snapshot_emissions: u128,

    pub first_deposit_at: i64,
    pub last_action_at: i64,

    pub _padding: [u8; 2],
    pub reserved: [u8; 32],
}

impl Miner {
    pub const LEN: usize = 32 // owner
        + 1 * 2               // stope_id, bump
        + 4                   // ticket_count
        + 8 * 4               // shares, deposited, withdrawn, pending_redemption
        + 8 * 4               // accrued_sustainable, accrued_emissions,
                              // claimed_sustainable, claimed_emissions
        + 16 * 2              // index_snapshot_sustainable, index_snapshot_emissions
        + 8 * 2               // first_deposit_at, last_action_at
        + 2                   // _padding
        + 32; // reserved

    /// Total settled yield.
    ///
    /// A convenience view, never a stored field: the two components are what
    /// the account holds, and anything that needs the sum computes it here so
    /// the split can never be lost by writing only the total.
    pub fn accrued_yield(&self) -> u64 {
        self.accrued_sustainable
            .saturating_add(self.accrued_emissions)
    }

    /// Move this position up to the stope's current indices, crediting each
    /// kind into its own balance.
    ///
    /// Called before every change to `shares`. Settling first is what makes
    /// the index model correct: yield already earned is banked against the old
    /// share count before a deposit or a redemption changes it.
    pub fn settle(&mut self, stope: &Stope) -> Result<(u64, u64)> {
        self.settle_indices(stope.yield_index_sustainable, stope.yield_index_emissions)
    }

    /// [`Miner::settle`] against raw index values.
    ///
    /// The instruction handlers call this form rather than passing a `&Stope`,
    /// so the stope account never has to be borrowed at the same time as the
    /// miner account is being mutated.
    pub fn settle_indices(
        &mut self,
        index_sustainable: u128,
        index_emissions: u128,
    ) -> Result<(u64, u64)> {
        let sustainable = pending_from_index(
            self.shares,
            index_sustainable,
            self.index_snapshot_sustainable,
        )?;
        let emissions =
            pending_from_index(self.shares, index_emissions, self.index_snapshot_emissions)?;

        if sustainable > 0 {
            self.accrued_sustainable = self
                .accrued_sustainable
                .checked_add(sustainable)
                .ok_or(LodzError::MathOverflow)?;
        }
        if emissions > 0 {
            self.accrued_emissions = self
                .accrued_emissions
                .checked_add(emissions)
                .ok_or(LodzError::MathOverflow)?;
        }

        self.index_snapshot_sustainable = index_sustainable;
        self.index_snapshot_emissions = index_emissions;

        Ok((sustainable, emissions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RiskProfile, YieldKind};

    fn miner() -> Miner {
        Miner::try_from_slice(&vec![0u8; Miner::LEN]).expect("decode")
    }

    fn stope() -> Stope {
        let mut s = Stope::try_from_slice(&vec![0u8; Stope::LEN]).expect("decode");
        s.risk_profile = RiskProfile::Balanced;
        s.stope_id = 1;
        s
    }

    #[test]
    fn settlement_credits_each_kind_to_its_own_balance() {
        let mut s = stope();
        let mut m = miner();

        s.total_shares = 100_000_000;
        m.shares = 100_000_000;

        s.accrue(YieldKind::Sustainable, 3_000_000, 1).unwrap();
        s.accrue(YieldKind::Emissions, 7_000_000, 2).unwrap();

        let (sus, emi) = m.settle(&s).unwrap();
        assert_eq!(sus, 3_000_000);
        assert_eq!(emi, 7_000_000);
        assert_eq!(m.accrued_sustainable, 3_000_000);
        assert_eq!(m.accrued_emissions, 7_000_000);
        assert_eq!(m.accrued_yield(), 10_000_000);

        // Settling twice must not pay twice.
        let (sus, emi) = m.settle(&s).unwrap();
        assert_eq!((sus, emi), (0, 0));
        assert_eq!(m.accrued_sustainable, 3_000_000);
        assert_eq!(m.accrued_emissions, 7_000_000);
    }

    #[test]
    fn a_late_depositor_does_not_receive_earlier_yield() {
        let mut s = stope();
        let mut early = miner();
        let mut late = miner();

        s.total_shares = 100_000_000;
        early.shares = 100_000_000;
        s.accrue(YieldKind::Sustainable, 1_000_000, 1).unwrap();

        // The late miner settles on entry, taking the current index as its
        // snapshot, then takes its share of everything after.
        late.settle(&s).unwrap();
        late.shares = 100_000_000;
        s.total_shares = 200_000_000;

        s.accrue(YieldKind::Sustainable, 2_000_000, 2).unwrap();

        early.settle(&s).unwrap();
        late.settle(&s).unwrap();

        assert_eq!(late.accrued_sustainable, 1_000_000);
        assert_eq!(early.accrued_sustainable, 2_000_000);
        // Nothing was created: 1_000_000 + 2_000_000 accrued, at most that
        // much is claimable.
        assert!(early.accrued_sustainable + late.accrued_sustainable <= 3_000_000);
    }

    #[test]
    fn a_queued_miner_stops_earning() {
        let mut s = stope();
        let mut m = miner();

        s.total_shares = 100_000_000;
        m.shares = 100_000_000;

        // Queue the whole position: shares burn now, principal waits.
        m.settle(&s).unwrap();
        m.shares = 0;
        s.total_shares = 0;
        m.pending_redemption = 100_000_000;

        // With no shares outstanding there is nobody to accrue to at all.
        assert!(s.accrue(YieldKind::Sustainable, 1_000_000, 3).is_err());

        // And once another depositor arrives, the queued miner takes none of it.
        s.total_shares = 50_000_000;
        s.accrue(YieldKind::Sustainable, 1_000_000, 4).unwrap();
        let (sus, emi) = m.settle(&s).unwrap();
        assert_eq!((sus, emi), (0, 0));
    }

    #[test]
    fn accrued_yield_is_a_view_over_the_two_stored_balances() {
        let mut m = miner();
        m.accrued_sustainable = 400;
        m.accrued_emissions = 600;
        assert_eq!(m.accrued_yield(), 1_000);

        // Saturating rather than wrapping, so a corrupted upgrade cannot
        // produce a nonsense total.
        m.accrued_sustainable = u64::MAX;
        assert_eq!(m.accrued_yield(), u64::MAX);
    }
}
