//! `deposit` -- the Adit, the way in.
//!
//! A depositor hands over some quantity of one registered BTC representation.
//! The program normalizes it into internal accounting units using the
//! conversion pinned on that Adit, mints shares against the stope, and moves
//! the tokens into the program-owned custody account for that asset.
//!
//! What a share is: a claim on the stope's principal pool. Shares are minted
//! at the pool's current ratio (`normalized * total_shares / total_deposits`)
//! rather than 1:1, so realized yield already booked into the pool accrues to
//! the depositors who were there for it and not to whoever deposits next.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::errors::LodzError;
use crate::events::Deposit;
use crate::math::{mul_div_floor, to_normalized};
use crate::state::*;

#[derive(Accounts)]
#[instruction(stope_id: u8)]
pub struct MakeDeposit<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    #[account(
        mut,
        seeds = [ADIT_SEED, asset_mint.key().as_ref()],
        bump = adit.bump,
        has_one = asset_mint @ LodzError::AditMintMismatch,
        constraint = adit.vault == adit_vault.key() @ LodzError::AditVaultMismatch,
        constraint = adit.token_program == token_program.key() @ LodzError::TokenProgramMismatch,
    )]
    pub adit: Box<Account<'info, Adit>>,

    #[account(
        mut,
        seeds = [STOPE_SEED, &stope_id.to_le_bytes()],
        bump = stope.bump,
    )]
    pub stope: Box<Account<'info, Stope>>,

    /// Created on first deposit into this stope. This is one of exactly two
    /// `init_if_needed` accounts in the program.
    #[account(
        init_if_needed,
        payer = depositor,
        space = 8 + Miner::LEN,
        seeds = [MINER_SEED, depositor.key().as_ref(), &stope_id.to_le_bytes()],
        bump
    )]
    pub miner: Box<Account<'info, Miner>>,

    pub asset_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = asset_mint,
        token::authority = depositor,
        token::token_program = token_program,
    )]
    pub depositor_token: Box<InterfaceAccount<'info, TokenAccount>>,

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
    pub system_program: Program<'info, System>,
}

pub fn deposit(ctx: Context<MakeDeposit>, stope_id: u8, amount: u64) -> Result<()> {
    require!(!ctx.accounts.vault_config.paused, LodzError::VaultPaused);
    require!(!ctx.accounts.stope.paused, LodzError::StopePaused);
    require!(!ctx.accounts.adit.paused, LodzError::AditPaused);
    require!(amount > 0, LodzError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;

    let normalized = to_normalized(
        amount,
        ctx.accounts.adit.conversion_num,
        ctx.accounts.adit.conversion_den,
    )?;
    require!(normalized > 0, LodzError::DepositBelowMinimum);

    // Deposit cap, in the asset's own units. Zero means uncapped.
    if ctx.accounts.adit.deposit_cap > 0 {
        let after = ctx
            .accounts
            .adit
            .total_deposited
            .checked_add(amount)
            .ok_or(LodzError::MathOverflow)?;
        require!(
            after <= ctx.accounts.adit.deposit_cap,
            LodzError::DepositCapExceeded
        );
    }

    // Shares at the pool's current ratio. A fresh pool, or one whose principal
    // has been fully redeemed, seeds at 1:1.
    let stope_total_shares = ctx.accounts.stope.total_shares;
    let stope_total_deposits = ctx.accounts.stope.total_deposits;
    let shares = if stope_total_shares == 0 || stope_total_deposits == 0 {
        normalized
    } else {
        mul_div_floor(normalized, stope_total_shares, stope_total_deposits)?
    };
    require!(shares > 0, LodzError::DepositBelowMinimum);

    let decimals = ctx.accounts.adit.decimals;
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.depositor_token.to_account_info(),
                mint: ctx.accounts.asset_mint.to_account_info(),
                to: ctx.accounts.adit_vault.to_account_info(),
                authority: ctx.accounts.depositor.to_account_info(),
            },
        ),
        amount,
        decimals,
    )?;

    let depositor_key = ctx.accounts.depositor.key();
    let is_new_position = ctx.accounts.miner.owner == Pubkey::default();

    if is_new_position {
        let miner = &mut ctx.accounts.miner;
        miner.owner = depositor_key;
        miner.stope_id = stope_id;
        miner.bump = ctx.bumps.miner;
        miner.first_deposit_at = now;
    } else {
        require_keys_eq!(
            ctx.accounts.miner.owner,
            depositor_key,
            LodzError::Unauthorized
        );
    }

    // Bank whatever the existing position has earned before the share count
    // changes. For a brand new position this only copies the stope's current
    // indices into the snapshot, which is what stops a fresh depositor from
    // claiming a share of yield that was accrued before they arrived.
    let index_sustainable = ctx.accounts.stope.yield_index_sustainable;
    let index_emissions = ctx.accounts.stope.yield_index_emissions;
    ctx.accounts
        .miner
        .settle_indices(index_sustainable, index_emissions)?;

    let miner = &mut ctx.accounts.miner;
    miner.shares = miner
        .shares
        .checked_add(shares)
        .ok_or(LodzError::MathOverflow)?;
    miner.deposited = miner
        .deposited
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    miner.last_action_at = now;
    let miner_shares = miner.shares;
    let miner_key = miner.key();

    let stope = &mut ctx.accounts.stope;
    stope.total_shares = stope
        .total_shares
        .checked_add(shares)
        .ok_or(LodzError::MathOverflow)?;
    stope.total_deposits = stope
        .total_deposits
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    if is_new_position {
        stope.miner_count = stope
            .miner_count
            .checked_add(1)
            .ok_or(LodzError::MathOverflow)?;
    }
    let stope_total_shares = stope.total_shares;
    let stope_total_deposits = stope.total_deposits;

    let adit = &mut ctx.accounts.adit;
    adit.total_deposited = adit
        .total_deposited
        .checked_add(amount)
        .ok_or(LodzError::MathOverflow)?;
    adit.total_normalized = adit
        .total_normalized
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    let adit_key = adit.key();
    let asset_mint = adit.asset_mint;

    let config = &mut ctx.accounts.vault_config;
    config.total_normalized_deposits = config
        .total_normalized_deposits
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;

    emit!(Deposit {
        owner: depositor_key,
        miner: miner_key,
        stope_id,
        adit: adit_key,
        asset_mint,
        amount,
        normalized_amount: normalized,
        shares_minted: shares,
        miner_shares,
        stope_total_shares,
        stope_total_deposits,
        timestamp: now,
    });

    Ok(())
}
