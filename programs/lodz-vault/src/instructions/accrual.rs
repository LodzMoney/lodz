//! `accrue_yield` -- booking realized yield against one seam.
//!
//! # This instruction moves tokens
//!
//! An accrual is not a number a keeper types in. The keeper transfers the
//! realized yield, denominated in the seam's own asset, into the same
//! program-owned custody account the principal sits in. The stope's
//! `total_deposits` grows by that amount while `total_shares` does not, so the
//! yield is realized by every existing share immediately and is paid out
//! through the ordinary redemption path. There is no separate yield claim to
//! forget to call, and no accrued balance that exists only as a promise.
//!
//! # What the per-source split is for
//!
//! Because the pool's value moves, the two `yield_index_*` accumulators are
//! not an entitlement ledger -- they are the attribution of that movement. A
//! miner can read exactly how much of their position's growth came from a
//! counterparty paying interest and how much came from an emission schedule
//! with a date on it. That is the number the Assay Board is built on, and it
//! is why nothing in this program ever adds the two together into a single
//! stored field.
//!
//! # What the chain can and cannot show here
//!
//! It shows that the tokens arrived, that they were attributed to one named
//! seam of one declared kind, and that the reporter was a keeper with a
//! slashable bond posted. It cannot show that the venue those tokens came from
//! is solvent. Where the yield came from is an assertion by a bonded party;
//! that the vault is now holding it is a fact.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::errors::LodzError;
use crate::events::YieldAccrued;
use crate::math::to_normalized;
use crate::state::*;

#[derive(Accounts)]
#[instruction(seam_id: u16, stope_id: u8)]
pub struct AccrueYield<'info> {
    #[account(mut)]
    pub reporter: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    /// The reporter's bond. `active` is checked in the handler, so a keeper
    /// whose bond has been slashed below the minimum stops being able to book
    /// yield at all.
    #[account(
        seeds = [KEEPER_SEED, reporter.key().as_ref()],
        bump = keeper.bump,
        constraint = keeper.authority == reporter.key() @ LodzError::KeeperAuthorityMismatch,
    )]
    pub keeper: Box<Account<'info, Keeper>>,

    #[account(
        mut,
        seeds = [SEAM_SEED, &seam_id.to_le_bytes()],
        bump = seam.bump,
        constraint = seam.stope_id == stope_id @ LodzError::SeamStopeMismatch,
        constraint = seam.asset_mint == asset_mint.key() @ LodzError::SeamAssetMismatch,
    )]
    pub seam: Box<Account<'info, Seam>>,

    #[account(
        mut,
        seeds = [STOPE_SEED, &stope_id.to_le_bytes()],
        bump = stope.bump,
    )]
    pub stope: Box<Account<'info, Stope>>,

    #[account(
        mut,
        seeds = [ADIT_SEED, asset_mint.key().as_ref()],
        bump = adit.bump,
        has_one = asset_mint @ LodzError::AditMintMismatch,
        constraint = adit.vault == adit_vault.key() @ LodzError::AditVaultMismatch,
        constraint = adit.token_program == token_program.key() @ LodzError::TokenProgramMismatch,
    )]
    pub adit: Box<Account<'info, Adit>>,

    pub asset_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = asset_mint,
        token::authority = reporter,
        token::token_program = token_program,
    )]
    pub reporter_token: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [ADIT_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        token::mint = asset_mint,
        token::authority = vault_config,
        token::token_program = token_program,
    )]
    pub adit_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn accrue_yield(
    ctx: Context<AccrueYield>,
    seam_id: u16,
    stope_id: u8,
    amount: u64,
) -> Result<()> {
    require!(!ctx.accounts.vault_config.paused, LodzError::VaultPaused);
    require!(ctx.accounts.keeper.active, LodzError::KeeperNotActive);
    require!(ctx.accounts.seam.active, LodzError::SeamInactive);
    require!(amount > 0, LodzError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;

    // An emissions seam stops being able to book yield the moment its declared
    // schedule ends. Without this the `emission_ends_at` field would be a
    // label rather than a commitment: a seam could keep reporting emissions
    // yield past the date it told depositors the emissions would stop.
    require!(
        ctx.accounts.seam.accrual_window_open(now),
        LodzError::EmissionEnded
    );

    let normalized = to_normalized(
        amount,
        ctx.accounts.adit.conversion_num,
        ctx.accounts.adit.conversion_den,
    )?;
    require!(normalized > 0, LodzError::DepositBelowMinimum);

    // Fails when the stope holds no shares. An accrual with nobody to
    // attribute it to would leave `realized_*` reporting yield that no miner
    // can ever realize, so it is rejected rather than absorbed.
    require!(
        ctx.accounts.stope.total_shares > 0,
        LodzError::NoSharesOutstanding
    );

    let decimals = ctx.accounts.adit.decimals;
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.reporter_token.to_account_info(),
                mint: ctx.accounts.asset_mint.to_account_info(),
                to: ctx.accounts.adit_vault.to_account_info(),
                authority: ctx.accounts.reporter.to_account_info(),
            },
        ),
        amount,
        decimals,
    )?;

    let yield_kind = ctx.accounts.seam.yield_kind;

    let seam = &mut ctx.accounts.seam;
    seam.realized_yield = seam
        .realized_yield
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    seam.accrual_count = seam
        .accrual_count
        .checked_add(1)
        .ok_or(LodzError::MathOverflow)?;
    seam.last_accrual_at = now;
    let seam_key = seam.key();
    let seam_realized_yield = seam.realized_yield;
    let emission_ends_at = seam.emission_ends_at;

    let stope = &mut ctx.accounts.stope;
    // Moves the lifetime total and the per-share index for this kind, and
    // touches neither for the other kind.
    stope.accrue(yield_kind, normalized, now)?;
    // Yield becomes backing. `total_shares` is untouched, so every existing
    // share is worth more from this point.
    stope.total_deposits = stope
        .total_deposits
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    let stope_realized_sustainable = stope.realized_sustainable;
    let stope_realized_emissions = stope.realized_emissions;
    let stope_total_shares = stope.total_shares;

    let adit = &mut ctx.accounts.adit;
    adit.total_deposited = adit
        .total_deposited
        .checked_add(amount)
        .ok_or(LodzError::MathOverflow)?;
    adit.total_normalized = adit
        .total_normalized
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;

    let config = &mut ctx.accounts.vault_config;
    config.total_normalized_deposits = config
        .total_normalized_deposits
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;

    emit!(YieldAccrued {
        seam: seam_key,
        seam_id,
        stope_id,
        reporter: ctx.accounts.reporter.key(),
        yield_kind,
        amount: normalized,
        seam_realized_yield,
        stope_realized_sustainable,
        stope_realized_emissions,
        stope_total_shares,
        emission_ends_at,
        timestamp: now,
    });

    Ok(())
}
