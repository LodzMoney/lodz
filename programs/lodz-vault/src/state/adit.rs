//! `Adit` -- the entrance, at `["adit", asset_mint]`.
//!
//! An adit is one accepted representation of bitcoin on Solana, plus the
//! numbers needed to convert it into the vault's internal accounting unit and
//! the disclosure of what the depositor is actually holding.
//!
//! There is one adit per mint, and a mint that has no adit cannot be
//! deposited. That is the whole gate: LODZ never accepts a token whose custody
//! model and risk tier have not been written down on-chain first.

use anchor_lang::prelude::*;

use crate::state::CustodyKind;

/// A registered BTC representation and its custody vault.
#[account]
pub struct Adit {
    /// The SPL Token or Token-2022 mint accepted here.
    pub asset_mint: Pubkey,
    /// Token program that owns `asset_mint`, pinned at registration.
    ///
    /// A BTC representation may be a classic SPL Token mint or a Token-2022
    /// mint. Every transfer on the deposit and payout paths re-checks the
    /// supplied token program against this field, which is what makes it
    /// impossible to route a Token-2022 mint through the classic program (the
    /// `AccountOwnedByWrongProgram` failure in
    /// new_project_guide/references/solana/anchor-lessons.md).
    pub token_program: Pubkey,
    /// `["adit_vault", asset_mint]` -- the program-owned custody token
    /// account for this asset, whose authority is the `VaultConfig` PDA.
    pub vault: Pubkey,

    /// Conversion into internal accounting units:
    /// `normalized = floor(amount * conversion_num / conversion_den)`.
    ///
    /// Folds two things together, both fixed by the authority at registration:
    /// the decimal difference between `decimals` and the internal 8-decimal
    /// unit, and the asset's declared ratio to one BTC. A representation that
    /// is not 1:1 with BTC is expressed here rather than being quietly counted
    /// as if it were.
    pub conversion_num: u64,
    pub conversion_den: u64,

    /// Hard cap on `total_deposited`, in the asset's native units. Zero means
    /// no cap.
    pub deposit_cap: u64,
    /// Native units currently custodied for this asset.
    pub total_deposited: u64,
    /// Internal accounting units credited against this asset over its
    /// lifetime. Never decreases; `total_deposited` does.
    pub total_normalized: u64,
    pub registered_at: i64,

    /// Short NUL-padded ASCII label, e.g. `zBTC`. For indexers and logs; the
    /// mint is the identity.
    pub label: [u8; 16],

    /// What the depositor is actually exposed to. See [`CustodyKind`]: every
    /// variant is a token on Solana standing in for bitcoin held elsewhere,
    /// never bitcoin on the Bitcoin network itself.
    pub custody_kind: CustodyKind,
    /// Layer severity, 1 (lowest) to 5 (highest).
    ///
    /// The same 1..5 scale `packages/headlamp-risk` uses per risk layer, not
    /// the `low`/`medium`/`high` band that package reports -- those are a
    /// presentation bucket over a composite of these. `docs/risk-spec.md` 2.4
    /// carries the conversion and the reason it only runs one way.
    pub risk_tier: u8,
    /// Decimals of `asset_mint`, cached so the payout path can pass them to
    /// `transfer_checked` without deserializing the mint twice.
    pub decimals: u8,
    /// Blocks new deposits through this adit. Redemptions of assets already
    /// custodied here continue to settle.
    pub paused: bool,
    pub bump: u8,

    pub _padding: [u8; 3],
    pub reserved: [u8; 32],
}

impl Adit {
    pub const LEN: usize = 32 * 3 // asset_mint, token_program, vault
        + 8 * 6                   // conversion_num, conversion_den, deposit_cap,
                                  // total_deposited, total_normalized, registered_at
        + 16                      // label
        + 1 * 5                   // custody_kind, risk_tier, decimals, paused, bump
        + 3                       // _padding
        + 32; // reserved
}
