//! `VaultConfig` -- the one global account, at `["vault_config"]`.

use anchor_lang::prelude::*;

/// Global protocol configuration and the signing authority for every
/// program-owned token account.
///
/// This PDA is also the token authority of `["bond_vault"]` and of every
/// `["adit_vault", asset_mint]`, so its bump is read on the payout path and is
/// stored rather than recomputed.
#[account]
pub struct VaultConfig {
    /// Admin key. Checked with `has_one = authority` on every admin-only
    /// instruction.
    pub authority: Pubkey,
    /// Proposed next authority. `Pubkey::default()` when no handover is
    /// pending. Two-step handover exists so a typo in an authority transfer
    /// cannot brick the protocol.
    pub pending_authority: Pubkey,
    /// $LODZ mint. Keeper bonds are denominated in it.
    pub lodz_mint: Pubkey,
    /// Token program that owns `lodz_mint`, pinned at initialization. Deposits
    /// pin their own token program per Adit; this one covers bond and slash.
    pub lodz_token_program: Pubkey,
    /// $LODZ token account that receives slashed keeper bonds.
    pub treasury: Pubkey,

    /// Redemption fee, in basis points, charged on the gross payout of an
    /// Orecart ticket. Capped by `MAX_FEE_BPS`.
    pub fee_bps: u16,

    /// Base wait stamped on every ticket, before queue congestion.
    pub redemption_delay_sec: i64,
    /// Ceiling applied to `base + congestion`. A ticket is never stamped with
    /// a `claimable_at` further out than this.
    pub max_redemption_delay_sec: i64,
    /// Internal accounting units the protocol commits to being able to settle
    /// per day. Drives the congestion term in `math::queue_delay_sec`.
    pub queue_drain_per_day: u64,
    /// Minimum bond for a keeper to count as active.
    pub min_keeper_bond: u64,
    /// A keeper may not withdraw bond until this many seconds have passed
    /// since its last rebalance. Evidence that a rebalance was bad surfaces
    /// after the fact, so the cooldown is anchored to the last action rather
    /// than to the unbond request.
    pub keeper_unbond_cooldown_sec: i64,
    /// Sum of live deposits across every stope, in internal accounting units.
    pub total_normalized_deposits: u64,

    pub adit_count: u16,
    pub seam_count: u16,
    pub keeper_count: u16,
    pub stope_count: u8,

    /// Global circuit breaker. Blocks deposits, accrual, redemption requests
    /// and rebalances. It deliberately does **not** block `claim_redemption`:
    /// a ticket whose delay has already elapsed is a settled debt, and a pause
    /// switch that can strand it is a rug with extra steps.
    pub paused: bool,
    pub bump: u8,

    pub _padding: [u8; 5],
    pub reserved: [u8; 64],
}

impl VaultConfig {
    pub const LEN: usize = 32 * 5 // authority, pending_authority, lodz_mint, lodz_token_program, treasury
        + 2                       // fee_bps
        + 8 * 6                   // redemption_delay_sec, max_redemption_delay_sec, queue_drain_per_day,
                                  // min_keeper_bond, keeper_unbond_cooldown_sec, total_normalized_deposits
        + 2 * 3                   // adit_count, seam_count, keeper_count
        + 1 * 3                   // stope_count, paused, bump
        + 5                       // _padding
        + 64; // reserved

    /// Seeds for signing a CPI as this PDA.
    pub fn signer_seeds<'a>(bump: &'a [u8; 1]) -> [&'a [u8]; 2] {
        [crate::state::VAULT_CONFIG_SEED, bump]
    }
}
