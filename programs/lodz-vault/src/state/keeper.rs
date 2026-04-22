//! `Keeper` -- a bonded Seam Router operator, at `["keeper", authority]`.
//!
//! Rebalancing between seams is the one thing in this protocol that the chain
//! cannot check for itself: whether moving capital from one venue to another
//! was the right call only becomes visible later. The bond is the answer. A
//! keeper posts $LODZ, `update_seam_allocation` requires an active bond, and
//! `slash_keeper` takes it. Nothing here claims a keeper is trustworthy; it
//! claims that being wrong is expensive.

use anchor_lang::prelude::*;

/// A keeper's bond and its record.
#[account]
pub struct Keeper {
    /// The keeper's signing key, and the second seed of this PDA.
    pub authority: Pubkey,

    /// $LODZ currently bonded, held in `["bond_vault"]`.
    pub bonded_amount: u64,
    /// $LODZ taken by slashes over this keeper's lifetime.
    pub slashed_amount: u64,
    pub rebalance_count: u64,

    pub bonded_at: i64,
    pub last_rebalance_at: i64,
    /// Earliest time this keeper may withdraw bond.
    ///
    /// Recomputed as `last_rebalance_at + keeper_unbond_cooldown_sec` on every
    /// rebalance, so a keeper cannot make a bad reallocation and withdraw its
    /// bond before the consequences of it are visible.
    pub unbond_ready_at: i64,

    pub slash_count: u32,
    /// True while `bonded_amount >= VaultConfig::min_keeper_bond`. Maintained
    /// on every bond, unbond and slash.
    pub active: bool,
    pub bump: u8,

    pub _padding: [u8; 2],
    pub reserved: [u8; 32],
}

impl Keeper {
    pub const LEN: usize = 32 // authority
        + 8 * 3               // bonded_amount, slashed_amount, rebalance_count
        + 8 * 3               // bonded_at, last_rebalance_at, unbond_ready_at
        + 4                   // slash_count
        + 1 * 2               // active, bump
        + 2                   // _padding
        + 32; // reserved

    /// Bring `active` in step with the bond, reporting whether it changed so
    /// the caller can keep `VaultConfig::keeper_count` accurate.
    ///
    /// Returns `Some(true)` when the keeper became active, `Some(false)` when
    /// it stopped being active, `None` when nothing changed.
    pub fn refresh_active(&mut self, min_bond: u64) -> Option<bool> {
        let should_be_active = self.bonded_amount >= min_bond && min_bond > 0;
        if should_be_active == self.active {
            return None;
        }
        self.active = should_be_active;
        Some(should_be_active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keeper() -> Keeper {
        Keeper::try_from_slice(&vec![0u8; Keeper::LEN]).expect("decode")
    }

    #[test]
    fn active_tracks_the_bond_across_the_minimum() {
        let mut k = keeper();
        let min = 1_000u64;

        assert_eq!(k.refresh_active(min), None, "starts inactive, stays inactive");

        k.bonded_amount = 999;
        assert_eq!(k.refresh_active(min), None);

        k.bonded_amount = 1_000;
        assert_eq!(k.refresh_active(min), Some(true));
        assert!(k.active);

        // Crossing the line more than once must not double-count.
        assert_eq!(k.refresh_active(min), None);

        // A slash that takes it below the minimum deactivates exactly once.
        k.bonded_amount = 1;
        assert_eq!(k.refresh_active(min), Some(false));
        assert!(!k.active);
        assert_eq!(k.refresh_active(min), None);
    }
}
