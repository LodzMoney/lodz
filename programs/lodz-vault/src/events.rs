//! One event per state transition.
//!
//! The service indexer in `apps/service` rebuilds the Seam Map, the Assay
//! Board and the Orecart queue view from these logs alone, so every field an
//! off-chain reader would otherwise have to infer is emitted explicitly --
//! including the running totals after the change, not just the delta.
//!
//! [`YieldAccrued`] carries `yield_kind` and the two post-accrual stope totals
//! side by side. An indexer that only ever sees these events can therefore
//! never produce a blended APY by accident: there is no field in this file
//! that adds sustainable and emissions yield together.

use anchor_lang::prelude::*;

use crate::state::{CustodyKind, RiskProfile, YieldKind};

#[event]
pub struct VaultInitialized {
    pub vault_config: Pubkey,
    pub authority: Pubkey,
    pub lodz_mint: Pubkey,
    pub treasury: Pubkey,
    pub fee_bps: u16,
    pub redemption_delay_sec: i64,
    pub max_redemption_delay_sec: i64,
    pub queue_drain_per_day: u64,
    pub min_keeper_bond: u64,
    pub timestamp: i64,
}

#[event]
pub struct BondVaultInitialized {
    pub vault_config: Pubkey,
    pub bond_vault: Pubkey,
    pub lodz_mint: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct AditRegistered {
    pub adit: Pubkey,
    pub asset_mint: Pubkey,
    pub vault: Pubkey,
    pub token_program: Pubkey,
    pub label: [u8; 16],
    pub custody_kind: CustodyKind,
    pub risk_tier: u8,
    pub decimals: u8,
    pub conversion_num: u64,
    pub conversion_den: u64,
    pub deposit_cap: u64,
    pub timestamp: i64,
}

#[event]
pub struct StopeOpened {
    pub stope: Pubkey,
    pub stope_id: u8,
    pub risk_profile: RiskProfile,
    pub max_emissions_bps: u16,
    pub max_risk_tier: u8,
    pub orecart_queue: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct SeamRegistered {
    pub seam: Pubkey,
    pub seam_id: u16,
    pub stope_id: u8,
    pub venue: [u8; 32],
    pub venue_program: Pubkey,
    pub asset_mint: Pubkey,
    pub yield_kind: YieldKind,
    pub allocation_bps: u16,
    pub risk_tier: u8,
    /// Zero for a sustainable seam; the end of the schedule for an emissions
    /// seam, which the program required to be in the future at registration.
    pub emission_ends_at: i64,
    pub emission_mint: Pubkey,
    /// Stope-wide emissions share after this seam was added.
    pub stope_emissions_bps: u16,
    pub timestamp: i64,
}

#[event]
pub struct SeamRebalanced {
    pub seam: Pubkey,
    pub seam_id: u16,
    pub stope_id: u8,
    pub keeper: Pubkey,
    pub yield_kind: YieldKind,
    pub previous_allocation_bps: u16,
    pub new_allocation_bps: u16,
    /// Stope totals after the change, so a reader never has to re-sum the
    /// seam accounts to see the resulting exposure.
    pub stope_allocated_bps: u16,
    pub stope_emissions_bps: u16,
    pub timestamp: i64,
}

#[event]
pub struct Deposit {
    pub owner: Pubkey,
    pub miner: Pubkey,
    pub stope_id: u8,
    pub adit: Pubkey,
    pub asset_mint: Pubkey,
    /// What the depositor actually transferred, in the asset's native units.
    pub amount: u64,
    /// What it was worth in internal accounting units.
    pub normalized_amount: u64,
    pub shares_minted: u64,
    pub miner_shares: u64,
    pub stope_total_shares: u64,
    pub stope_total_deposits: u64,
    pub timestamp: i64,
}

#[event]
pub struct YieldAccrued {
    pub seam: Pubkey,
    pub seam_id: u16,
    pub stope_id: u8,
    pub reporter: Pubkey,
    /// The whole point of this event. Never summed with the other kinds.
    pub yield_kind: YieldKind,
    pub amount: u64,
    pub seam_realized_yield: u64,
    /// Stope lifetime totals after this accrual, kept apart. An indexer that
    /// wants a blended number has to add these itself and say that it did.
    pub stope_realized_sustainable: u64,
    pub stope_realized_emissions: u64,
    pub stope_realized_counterparty: u64,
    pub stope_total_shares: u64,
    /// Zero for a sustainable seam. For an emissions seam this is when the
    /// number above stops arriving.
    pub emission_ends_at: i64,
    pub timestamp: i64,
}

#[event]
pub struct RedemptionRequested {
    pub owner: Pubkey,
    pub orecart: Pubkey,
    pub ticket_index: u32,
    pub stope_id: u8,
    pub asset_mint: Pubkey,
    pub shares_burned: u64,
    pub normalized_amount: u64,
    pub fee_bps: u16,
    pub fee_normalized: u64,
    /// The realized yield the fee was charged on. The fee never touches
    /// `principal_normalized`.
    pub fee_basis_normalized: u64,
    /// `normalized_amount - fee_basis_normalized`. This comes back one for
    /// one; publishing it beside the fee is what makes that checkable from the
    /// event stream alone.
    pub principal_normalized: u64,
    /// The fee basis, split by where the yield came from.
    pub claimed_sustainable: u64,
    pub claimed_emissions: u64,
    pub claimed_counterparty: u64,
    pub gross_amount: u64,
    pub payout_amount: u64,
    pub queue_position: u64,
    /// Backlog standing in front of this ticket when it was issued.
    pub queue_pending_ahead: u64,
    pub requested_at: i64,
    /// The enforced gate, not an estimate.
    pub claimable_at: i64,
}

#[event]
pub struct RedemptionClaimed {
    pub owner: Pubkey,
    pub orecart: Pubkey,
    pub ticket_index: u32,
    pub stope_id: u8,
    pub asset_mint: Pubkey,
    pub payout_amount: u64,
    pub fee_amount: u64,
    pub normalized_amount: u64,
    /// How long the depositor actually waited, in seconds.
    pub waited_sec: i64,
    pub queue_total_pending: u64,
    pub claimed_at: i64,
}

#[event]
pub struct KeeperBonded {
    pub keeper: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
    pub bonded_amount: u64,
    pub active: bool,
    pub keeper_count: u16,
    pub timestamp: i64,
}

#[event]
pub struct KeeperUnbonded {
    pub keeper: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
    pub bonded_amount: u64,
    pub active: bool,
    pub keeper_count: u16,
    pub timestamp: i64,
}

#[event]
pub struct KeeperSlashed {
    pub keeper: Pubkey,
    pub authority: Pubkey,
    pub slashed_by: Pubkey,
    pub amount: u64,
    pub bonded_amount: u64,
    pub slash_count: u32,
    /// Free-form reason code recorded on-chain so a slash can be argued about
    /// with a reference rather than from memory.
    pub reason_code: u16,
    pub active: bool,
    pub timestamp: i64,
}

#[event]
pub struct VaultPauseChanged {
    pub vault_config: Pubkey,
    pub authority: Pubkey,
    pub paused: bool,
    pub timestamp: i64,
}

#[event]
pub struct AuthorityTransferProposed {
    pub vault_config: Pubkey,
    pub authority: Pubkey,
    pub pending_authority: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct AuthorityTransferAccepted {
    pub vault_config: Pubkey,
    pub previous_authority: Pubkey,
    pub authority: Pubkey,
    pub timestamp: i64,
}
