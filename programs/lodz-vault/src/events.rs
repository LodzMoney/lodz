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
    /// The whole point of this event. Never summed with the other kind.
    pub yield_kind: YieldKind,
    pub amount: u64,
    pub seam_realized_yield: u64,
    /// Stope lifetime totals after this accrual, kept apart.
    pub stope_realized_sustainable: u64,
    pub stope_realized_emissions: u64,
    pub stope_total_shares: u64,
    /// Zero for a sustainable seam. For an emissions seam this is when the
    /// number above stops arriving.
    pub emission_ends_at: i64,
    pub timestamp: i64,
}
