//! Authority-only instructions: protocol setup, the Adit and Seam registries,
//! the circuit breaker, keeper slashing and authority handover.
//!
//! Every account struct in this file checks the signer with
//! `has_one = authority`. Every parameter the authority can set is bounded by
//! a constant in `state`, so a compromised authority key can degrade the
//! protocol but cannot, for example, set a 100 % redemption fee or a ten-year
//! redemption delay.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::errors::LodzError;
use crate::events::*;
use crate::math::{validate_ascii_label, MAX_BPS};
use crate::state::*;

// ---------------------------------------------------------------------------
// initialize_vault
// ---------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitializeVaultParams {
    /// Redemption fee in basis points. Bounded by `MAX_FEE_BPS`.
    pub fee_bps: u16,
    /// Base wait stamped on every Orecart ticket.
    pub redemption_delay_sec: i64,
    /// Ceiling on base + congestion.
    pub max_redemption_delay_sec: i64,
    /// Internal accounting units the protocol commits to settling per day.
    pub queue_drain_per_day: u64,
    /// Minimum $LODZ bond for a keeper to count as active.
    pub min_keeper_bond: u64,
    /// How long after its last rebalance a keeper must wait to unbond.
    pub keeper_unbond_cooldown_sec: i64,
}

impl InitializeVaultParams {
    pub fn validate(&self) -> Result<()> {
        require!(self.fee_bps <= MAX_FEE_BPS, LodzError::InvalidFeeBps);

        require!(
            self.redemption_delay_sec >= 0
                && self.redemption_delay_sec <= MAX_BASE_REDEMPTION_DELAY_SEC,
            LodzError::InvalidDelay
        );
        require!(
            self.max_redemption_delay_sec >= self.redemption_delay_sec
                && self.max_redemption_delay_sec <= MAX_TOTAL_REDEMPTION_DELAY_SEC,
            LodzError::InvalidDelay
        );
        require!(
            self.queue_drain_per_day > 0,
            LodzError::InvalidQueueDrain
        );
        require!(self.min_keeper_bond > 0, LodzError::InvalidMinKeeperBond);
        require!(
            self.keeper_unbond_cooldown_sec >= 0
                && self.keeper_unbond_cooldown_sec <= MAX_KEEPER_UNBOND_COOLDOWN_SEC,
            LodzError::InvalidDelay
        );
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + VaultConfig::LEN,
        seeds = [VAULT_CONFIG_SEED],
        bump
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    pub lodz_mint: Box<InterfaceAccount<'info, Mint>>,

    /// $LODZ destination for slashed keeper bonds. Recorded now so
    /// `slash_keeper` cannot be pointed at an arbitrary account later.
    #[account(
        token::mint = lodz_mint,
        token::token_program = lodz_token_program,
    )]
    pub treasury: Box<InterfaceAccount<'info, TokenAccount>>,

    pub lodz_token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn initialize_vault(
    ctx: Context<InitializeVault>,
    params: InitializeVaultParams,
) -> Result<()> {
    params.validate()?;

    let now = Clock::get()?.unix_timestamp;
    let config = &mut ctx.accounts.vault_config;

    config.authority = ctx.accounts.authority.key();
    config.pending_authority = Pubkey::default();
    config.lodz_mint = ctx.accounts.lodz_mint.key();
    config.lodz_token_program = ctx.accounts.lodz_token_program.key();
    config.treasury = ctx.accounts.treasury.key();

    config.fee_bps = params.fee_bps;
    config.redemption_delay_sec = params.redemption_delay_sec;
    config.max_redemption_delay_sec = params.max_redemption_delay_sec;
    config.queue_drain_per_day = params.queue_drain_per_day;
    config.min_keeper_bond = params.min_keeper_bond;
    config.keeper_unbond_cooldown_sec = params.keeper_unbond_cooldown_sec;

    config.total_normalized_deposits = 0;
    config.adit_count = 0;
    config.seam_count = 0;
    config.keeper_count = 0;
    config.stope_count = 0;

    // The vault starts paused. No stope exists yet, no adit exists yet, and a
    // deposit into that state would be accepted against nothing. Unpausing is
    // a deliberate second transaction.
    config.paused = true;
    config.bump = ctx.bumps.vault_config;

    emit!(VaultInitialized {
        vault_config: config.key(),
        authority: config.authority,
        lodz_mint: config.lodz_mint,
        treasury: config.treasury,
        fee_bps: config.fee_bps,
        redemption_delay_sec: config.redemption_delay_sec,
        max_redemption_delay_sec: config.max_redemption_delay_sec,
        queue_drain_per_day: config.queue_drain_per_day,
        min_keeper_bond: config.min_keeper_bond,
        timestamp: now,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// initialize_bond_vault
//
// Split out of initialize_vault: `init` on a token account expands into a
// large amount of stack-resident CPI setup, and packing it beside the config
// init is how the "Stack offset exceeded max offset" failure in
// references/solana/anchor-lessons.md reproduces.
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeBondVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
        has_one = authority @ LodzError::Unauthorized,
        has_one = lodz_mint @ LodzError::AditMintMismatch,
        constraint = token_program.key() == vault_config.lodz_token_program
            @ LodzError::TokenProgramMismatch,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    pub lodz_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        init,
        payer = authority,
        seeds = [BOND_VAULT_SEED],
        bump,
        token::mint = lodz_mint,
        token::authority = vault_config,
        token::token_program = token_program,
    )]
    pub bond_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn initialize_bond_vault(ctx: Context<InitializeBondVault>) -> Result<()> {
    emit!(BondVaultInitialized {
        vault_config: ctx.accounts.vault_config.key(),
        bond_vault: ctx.accounts.bond_vault.key(),
        lodz_mint: ctx.accounts.lodz_mint.key(),
        timestamp: Clock::get()?.unix_timestamp,
    });
    Ok(())
}
