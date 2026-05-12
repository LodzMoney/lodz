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

// ---------------------------------------------------------------------------
// register_adit
// ---------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct RegisterAditParams {
    /// NUL-padded ASCII, e.g. `zBTC`.
    pub label: [u8; 16],
    /// What the depositor is exposed to. None of the variants is native
    /// bitcoin.
    pub custody_kind: CustodyKind,
    /// Headlamp tier, 1..=5.
    pub risk_tier: u8,
    /// `normalized = floor(amount * conversion_num / conversion_den)`.
    pub conversion_num: u64,
    pub conversion_den: u64,
    /// Cap on native units custodied here. Zero means uncapped.
    pub deposit_cap: u64,
}

#[derive(Accounts)]
pub struct RegisterAdit<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
        has_one = authority @ LodzError::Unauthorized,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    pub asset_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        init,
        payer = authority,
        space = 8 + Adit::LEN,
        seeds = [ADIT_SEED, asset_mint.key().as_ref()],
        bump
    )]
    pub adit: Box<Account<'info, Adit>>,

    #[account(
        init,
        payer = authority,
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

pub fn register_adit(ctx: Context<RegisterAdit>, params: RegisterAditParams) -> Result<()> {
    require!(
        params.conversion_num != 0 && params.conversion_den != 0,
        LodzError::InvalidConversionRatio
    );
    require!(
        params.risk_tier >= MIN_RISK_TIER && params.risk_tier <= MAX_RISK_TIER,
        LodzError::InvalidRiskTier
    );
    require!(
        validate_ascii_label(&params.label),
        LodzError::InvalidLabel
    );

    let decimals = ctx.accounts.asset_mint.decimals;
    require!(decimals <= MAX_MINT_DECIMALS, LodzError::InvalidDecimals);

    let now = Clock::get()?.unix_timestamp;

    let adit = &mut ctx.accounts.adit;
    adit.asset_mint = ctx.accounts.asset_mint.key();
    adit.token_program = ctx.accounts.token_program.key();
    adit.vault = ctx.accounts.adit_vault.key();
    adit.conversion_num = params.conversion_num;
    adit.conversion_den = params.conversion_den;
    adit.deposit_cap = params.deposit_cap;
    adit.total_deposited = 0;
    adit.total_normalized = 0;
    adit.registered_at = now;
    adit.label = params.label;
    adit.custody_kind = params.custody_kind;
    adit.risk_tier = params.risk_tier;
    adit.decimals = decimals;
    adit.paused = false;
    adit.bump = ctx.bumps.adit;

    let config = &mut ctx.accounts.vault_config;
    config.adit_count = config
        .adit_count
        .checked_add(1)
        .ok_or(LodzError::MathOverflow)?;

    emit!(AditRegistered {
        adit: adit.key(),
        asset_mint: adit.asset_mint,
        vault: adit.vault,
        token_program: adit.token_program,
        label: adit.label,
        custody_kind: adit.custody_kind,
        risk_tier: adit.risk_tier,
        decimals: adit.decimals,
        conversion_num: adit.conversion_num,
        conversion_den: adit.conversion_den,
        deposit_cap: adit.deposit_cap,
        timestamp: now,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// open_stope
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(stope_id: u8)]
pub struct OpenStope<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
        has_one = authority @ LodzError::Unauthorized,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    #[account(
        init,
        payer = authority,
        space = 8 + Stope::LEN,
        seeds = [STOPE_SEED, &stope_id.to_le_bytes()],
        bump
    )]
    pub stope: Box<Account<'info, Stope>>,

    /// The stope and its queue are created together: a stope that can take
    /// deposits before its redemption queue exists is a stope you can get into
    /// and not out of.
    #[account(
        init,
        payer = authority,
        space = 8 + OrecartQueue::LEN,
        seeds = [ORECART_QUEUE_SEED, &stope_id.to_le_bytes()],
        bump
    )]
    pub orecart_queue: Box<Account<'info, OrecartQueue>>,

    pub system_program: Program<'info, System>,
}

pub fn open_stope(ctx: Context<OpenStope>, stope_id: u8, risk_profile: RiskProfile) -> Result<()> {
    let canonical = RiskProfile::from_id(stope_id).ok_or(LodzError::InvalidStopeId)?;
    require!(
        canonical == risk_profile,
        LodzError::RiskProfileMismatch
    );

    let now = Clock::get()?.unix_timestamp;

    let stope = &mut ctx.accounts.stope;
    stope.stope_id = stope_id;
    stope.risk_profile = risk_profile;
    stope.paused = false;
    stope.bump = ctx.bumps.stope;
    stope.created_at = now;

    let queue = &mut ctx.accounts.orecart_queue;
    queue.stope_id = stope_id;
    queue.bump = ctx.bumps.orecart_queue;

    let config = &mut ctx.accounts.vault_config;
    config.stope_count = config
        .stope_count
        .checked_add(1)
        .ok_or(LodzError::MathOverflow)?;
    require!(config.stope_count <= STOPE_COUNT, LodzError::InvalidStopeId);

    emit!(StopeOpened {
        stope: stope.key(),
        stope_id,
        risk_profile,
        max_emissions_bps: risk_profile.max_emissions_bps(),
        max_risk_tier: risk_profile.max_risk_tier(),
        orecart_queue: queue.key(),
        timestamp: now,
    });

    Ok(())
}
