//! On-chain accounts, PDA seeds and the bounds the program enforces on its
//! own configuration.
//!
//! Every account carries an explicit `LEN` and a `reserved` tail. The tail is
//! upgrade headroom: an Anchor account layout can grow into trailing reserved
//! bytes without a migration, but it can never shrink or reorder, so the space
//! is claimed while it is still free. `space = 8 + T::LEN`, where 8 is the
//! Anchor discriminator.
//!
//! The `LEN` arithmetic is asserted against real Borsh output in
//! `tests::declared_lengths_match_the_structs`. A `LEN` that disagrees with
//! its struct silently corrupts every byte written past the end of the
//! account.

use anchor_lang::prelude::*;

pub mod adit;
pub mod config;
pub mod keeper;
pub mod miner;
pub mod orecart;
pub mod seam;
pub mod stope;

pub use adit::*;
pub use config::*;
pub use keeper::*;
pub use miner::*;
pub use orecart::*;
pub use seam::*;
pub use stope::*;

// ---------------------------------------------------------------------------
// PDA seeds
//
// Every seed is a distinct byte string. Numeric seeds are always
// little-endian (`to_le_bytes`): the on-chain program, the TypeScript SDK, the
// Python tooling and the service indexer must derive the same address byte for
// byte, and a big-endian slip is the PDA mismatch recorded in
// new_project_guide/references/solana/anchor-lessons.md.
// ---------------------------------------------------------------------------

/// `["vault_config"]`
pub const VAULT_CONFIG_SEED: &[u8] = b"vault_config";
/// `["adit", asset_mint]`
pub const ADIT_SEED: &[u8] = b"adit";
/// `["adit_vault", asset_mint]` -- program-owned custody token account
pub const ADIT_VAULT_SEED: &[u8] = b"adit_vault";
/// `["bond_vault"]` -- program-owned $LODZ keeper bond token account
pub const BOND_VAULT_SEED: &[u8] = b"bond_vault";
/// `["stope", stope_id(u8)]`
pub const STOPE_SEED: &[u8] = b"stope";
/// `["seam", seam_id(u16 LE)]`
pub const SEAM_SEED: &[u8] = b"seam";
/// `["miner", owner, stope_id(u8)]`
pub const MINER_SEED: &[u8] = b"miner";
/// `["orecart", owner, ticket_index(u32 LE)]`
pub const ORECART_SEED: &[u8] = b"orecart";
/// `["orecart_queue", stope_id(u8)]`
pub const ORECART_QUEUE_SEED: &[u8] = b"orecart_queue";
/// `["keeper", authority]`
pub const KEEPER_SEED: &[u8] = b"keeper";

// ---------------------------------------------------------------------------
// Bounds enforced in code rather than by policy
//
// These are ceilings the authority cannot raise by sending a transaction. A
// compromised authority key is a realistic failure mode for a young protocol,
// and these are the parameters where the difference between "bad" and
// "unrecoverable" is decided.
// ---------------------------------------------------------------------------

/// Redemption fee ceiling: 5 %. The authority sets the live value below this.
pub const MAX_FEE_BPS: u16 = 500;

/// A base redemption delay above 30 days is not a queue, it is a lockup, and
/// this product does not sell a lockup.
pub const MAX_BASE_REDEMPTION_DELAY_SEC: i64 = 30 * 86_400;

/// Hard ceiling on the *total* delay a ticket can be stamped with, including
/// the queue congestion term: 180 days.
pub const MAX_TOTAL_REDEMPTION_DELAY_SEC: i64 = 180 * 86_400;

/// Keeper unbond cooldown ceiling: 30 days.
pub const MAX_KEEPER_UNBOND_COOLDOWN_SEC: i64 = 30 * 86_400;

/// There are exactly three stopes: conservative, balanced, aggressive.
pub const STOPE_COUNT: u8 = 3;

/// Internal accounting unit: 8 decimals, i.e. one satoshi-equivalent.
///
/// Nothing about this makes a deposit "bitcoin". It is the unit LODZ keeps its
/// books in so that several different tokenized representations of BTC, each
/// with its own decimals and its own ratio to one BTC, can be added up at all.
pub const INTERNAL_DECIMALS: u8 = 8;

/// SPL and Token-2022 mints in scope carry 0..=18 decimals.
pub const MAX_MINT_DECIMALS: u8 = 18;

/// Headlamp risk tiers run 1 (lowest) to 5 (highest). There is no tier 0:
/// every representation of BTC on Solana carries bridge or custody risk, and a
/// zero would read as "none".
pub const MIN_RISK_TIER: u8 = 1;
pub const MAX_RISK_TIER: u8 = 5;
