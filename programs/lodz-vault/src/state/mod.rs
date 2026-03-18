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
