//! Orecart -- the redemption queue.
//!
//! `Orecart` is one ticket, at
//! `["orecart", owner, stope_id(u8), ticket_index(u32 LE)]`.
//! `OrecartQueue` is the per-stope aggregate, at
//! `["orecart_queue", stope_id(u8)]`.
//!
//! The queue is a delay queue, not a strict FIFO. Each ticket is stamped at
//! request time with its own `claimable_at`, computed from the base delay plus
//! the backlog standing in front of it, and `claim_redemption` refuses to run
//! before it. Two tickets never contend for the same slot, and nobody has to
//! be able to jump the line for an earlier ticket to settle first -- the wait
//! each depositor was quoted is the wait the chain enforces on them
//! individually.

use anchor_lang::prelude::*;

use crate::state::TicketStatus;

/// One redemption ticket. Immutable after issue except for its claim fields.
#[account]
pub struct Orecart {
    pub owner: Pubkey,
    /// Asset this ticket pays out in, fixed at request time.
    pub asset_mint: Pubkey,

    /// Index within the owner's tickets. Part of the PDA seed.
    pub ticket_index: u32,
    pub stope_id: u8,
    pub status: TicketStatus,
    pub bump: u8,
    /// Fee rate captured at request time, so a later fee change by the
    /// authority cannot be applied retroactively to a ticket already in the
    /// queue.
    pub fee_bps: u16,

    /// Shares burned to open this ticket. They are gone from the stope's share
    /// pool already; this field is the record, not a reservation.
    pub shares_burned: u64,
    /// Principal owed, in internal accounting units, before the fee.
    pub normalized_amount: u64,
    /// Fee, in internal accounting units.
    pub fee_normalized: u64,
    /// `normalized_amount` converted into the asset's native units.
    pub gross_amount: u64,
    /// Fee in native units: `gross_amount - payout_amount`.
    pub fee_amount: u64,
    /// What `claim_redemption` transfers, in native units.
    pub payout_amount: u64,
    /// Position in the stope's issue order.
    pub queue_position: u64,

    pub requested_at: i64,
    /// The chain-enforced gate. `claim_redemption` requires
    /// `Clock::get()?.unix_timestamp >= claimable_at`.
    pub claimable_at: i64,
    pub claimed_at: i64,

    // -- appended 2026-08-16, into the reserved tail --------------------------
    /// What `fee_normalized` was actually charged on: the realized yield
    /// attributable to `shares_burned`, and nothing else.
    ///
    /// `normalized_amount - fee_basis_normalized` is the principal in this
    /// ticket, and the fee cannot reach it. Recorded rather than derived so
    /// that "principal is returned one for one" is auditable from the ticket
    /// alone, without replaying the position's accrual history.
    pub fee_basis_normalized: u64,

    pub _padding: [u8; 7],
    pub reserved: [u8; 24],
}

impl Orecart {
    pub const LEN: usize = 32 * 2 // owner, asset_mint
        + 4                       // ticket_index
        + 1 * 3                   // stope_id, status, bump
        + 2                       // fee_bps
        + 8 * 7                   // shares_burned, normalized_amount, fee_normalized,
                                  // gross_amount, fee_amount, payout_amount, queue_position
        + 8 * 3                   // requested_at, claimable_at, claimed_at
        + 8                       // fee_basis_normalized
        + 7                       // _padding
        + 24; // reserved (was 32; 8 bytes went to fee_basis_normalized)

    pub fn is_claimable_at(&self, now: i64) -> bool {
        self.status == TicketStatus::Queued && now >= self.claimable_at
    }
}

/// Per-stope queue aggregate.
#[account]
pub struct OrecartQueue {
    pub stope_id: u8,
    pub bump: u8,

    /// Tickets claimed so far. Because tickets settle independently this is a
    /// count, not a pointer into an ordered list.
    pub head: u64,
    /// Tickets issued so far. The next ticket's `queue_position`.
    pub tail: u64,
    /// `tail - head`, maintained rather than derived so a reader sees it in
    /// one field.
    pub pending_tickets: u64,

    /// Internal units awaiting claim. This is the backlog the congestion term
    /// in `math::queue_delay_sec` is computed against.
    pub total_pending: u64,
    /// Internal units paid out over the queue's lifetime.
    pub total_claimed: u64,
    /// Internal units retained as redemption fees.
    pub total_fees: u64,

    pub last_request_at: i64,
    pub last_claim_at: i64,

    pub _padding: [u8; 6],
    pub reserved: [u8; 32],
}

impl OrecartQueue {
    pub const LEN: usize = 1 * 2 // stope_id, bump
        + 8 * 6                  // head, tail, pending_tickets, total_pending,
                                 // total_claimed, total_fees
        + 8 * 2                  // last_request_at, last_claim_at
        + 6                      // _padding
        + 32; // reserved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> Orecart {
        Orecart::try_from_slice(&vec![0u8; Orecart::LEN]).expect("decode")
    }

    #[test]
    fn a_queued_ticket_is_not_claimable_before_its_stamp() {
        let mut t = ticket();
        t.status = TicketStatus::Queued;
        t.claimable_at = 1_000;

        assert!(!t.is_claimable_at(999));
        assert!(t.is_claimable_at(1_000));
        assert!(t.is_claimable_at(1_001));
    }

    #[test]
    fn a_claimed_ticket_is_never_claimable_again() {
        let mut t = ticket();
        t.status = TicketStatus::Claimed;
        t.claimable_at = 1_000;

        assert!(!t.is_claimable_at(1_000));
        assert!(!t.is_claimable_at(i64::MAX));
    }
}
