//! Keeper bonding, unbonding, and the one thing a bond buys the right to do:
//! move a seam's allocation.
//!
//! Rebalancing is the only action in this protocol whose correctness the chain
//! cannot judge at the time it happens. Whether shifting capital out of one
//! venue and into another was right becomes visible later, from outside. So
//! the design does not try to verify it. It makes the operator post $LODZ,
//! requires an active bond on every allocation change, holds that bond for a
//! cooldown measured from the operator's last change rather than from its
//! withdrawal request, and lets the authority take it with `slash_keeper`.
//!
//! Nothing here asserts that keepers are honest. It asserts that a keeper who
//! is not has something to lose.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::errors::LodzError;
use crate::events::{KeeperBonded, KeeperUnbonded, SeamRebalanced};
use crate::math::MAX_BPS;
use crate::state::*;

// ---------------------------------------------------------------------------
// bond_keeper
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct BondKeeper<'info> {
    #[account(mut)]
    pub keeper_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
        has_one = lodz_mint @ LodzError::AditMintMismatch,
        constraint = token_program.key() == vault_config.lodz_token_program
            @ LodzError::TokenProgramMismatch,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    /// Created on first bond. The second of exactly two `init_if_needed`
    /// accounts in the program. No `has_one` here: on the creating call the
    /// account is still zeroed, and the PDA seed already binds it to the
    /// signer.
    #[account(
        init_if_needed,
        payer = keeper_authority,
        space = 8 + Keeper::LEN,
        seeds = [KEEPER_SEED, keeper_authority.key().as_ref()],
        bump
    )]
    pub keeper: Box<Account<'info, Keeper>>,

    pub lodz_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = lodz_mint,
        token::authority = keeper_authority,
        token::token_program = token_program,
    )]
    pub keeper_token: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
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

pub fn bond_keeper(ctx: Context<BondKeeper>, amount: u64) -> Result<()> {
    require!(amount > 0, LodzError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;
    let authority_key = ctx.accounts.keeper_authority.key();

    if ctx.accounts.keeper.authority == Pubkey::default() {
        let keeper = &mut ctx.accounts.keeper;
        keeper.authority = authority_key;
        keeper.bump = ctx.bumps.keeper;
        keeper.bonded_at = now;
    } else {
        require_keys_eq!(
            ctx.accounts.keeper.authority,
            authority_key,
            LodzError::KeeperAuthorityMismatch
        );
    }

    let decimals = ctx.accounts.lodz_mint.decimals;
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.keeper_token.to_account_info(),
                mint: ctx.accounts.lodz_mint.to_account_info(),
                to: ctx.accounts.bond_vault.to_account_info(),
                authority: ctx.accounts.keeper_authority.to_account_info(),
            },
        ),
        amount,
        decimals,
    )?;

    let min_bond = ctx.accounts.vault_config.min_keeper_bond;

    let keeper = &mut ctx.accounts.keeper;
    keeper.bonded_amount = keeper
        .bonded_amount
        .checked_add(amount)
        .ok_or(LodzError::MathOverflow)?;
    let transition = keeper.refresh_active(min_bond);
    let keeper_key = keeper.key();
    let bonded_amount = keeper.bonded_amount;
    let active = keeper.active;

    let config = &mut ctx.accounts.vault_config;
    apply_active_transition(config, transition)?;

    emit!(KeeperBonded {
        keeper: keeper_key,
        authority: authority_key,
        amount,
        bonded_amount,
        active,
        keeper_count: config.keeper_count,
        timestamp: now,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// unbond_keeper
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct UnbondKeeper<'info> {
    pub keeper_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
        has_one = lodz_mint @ LodzError::AditMintMismatch,
        constraint = token_program.key() == vault_config.lodz_token_program
            @ LodzError::TokenProgramMismatch,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    #[account(
        mut,
        seeds = [KEEPER_SEED, keeper_authority.key().as_ref()],
        bump = keeper.bump,
        constraint = keeper.authority == keeper_authority.key()
            @ LodzError::KeeperAuthorityMismatch,
    )]
    pub keeper: Box<Account<'info, Keeper>>,

    pub lodz_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        token::mint = lodz_mint,
        token::authority = keeper_authority,
        token::token_program = token_program,
    )]
    pub keeper_token: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [BOND_VAULT_SEED],
        bump,
        token::mint = lodz_mint,
        token::authority = vault_config,
        token::token_program = token_program,
    )]
    pub bond_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn unbond_keeper(ctx: Context<UnbondKeeper>, amount: u64) -> Result<()> {
    require!(amount > 0, LodzError::ZeroAmount);
    require!(
        amount <= ctx.accounts.keeper.bonded_amount,
        LodzError::InsufficientBond
    );

    let now = Clock::get()?.unix_timestamp;
    // Anchored to the last rebalance, not to this request: a keeper must not
    // be able to make a bad allocation change and withdraw before anyone can
    // reconcile it.
    require!(
        now >= ctx.accounts.keeper.unbond_ready_at,
        LodzError::KeeperUnbondCooldown
    );

    let decimals = ctx.accounts.lodz_mint.decimals;
    let config_bump = [ctx.accounts.vault_config.bump];
    let seeds = VaultConfig::signer_seeds(&config_bump);
    let signer_seeds: &[&[&[u8]]] = &[&seeds];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.bond_vault.to_account_info(),
                mint: ctx.accounts.lodz_mint.to_account_info(),
                to: ctx.accounts.keeper_token.to_account_info(),
                authority: ctx.accounts.vault_config.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        decimals,
    )?;

    let min_bond = ctx.accounts.vault_config.min_keeper_bond;
    let authority_key = ctx.accounts.keeper_authority.key();

    let keeper = &mut ctx.accounts.keeper;
    keeper.bonded_amount = keeper.bonded_amount.saturating_sub(amount);
    let transition = keeper.refresh_active(min_bond);
    let keeper_key = keeper.key();
    let bonded_amount = keeper.bonded_amount;
    let active = keeper.active;

    let config = &mut ctx.accounts.vault_config;
    apply_active_transition(config, transition)?;

    emit!(KeeperUnbonded {
        keeper: keeper_key,
        authority: authority_key,
        amount,
        bonded_amount,
        active,
        keeper_count: config.keeper_count,
        timestamp: now,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// update_seam_allocation
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(seam_id: u16, stope_id: u8)]
pub struct UpdateSeamAllocation<'info> {
    pub keeper_authority: Signer<'info>,

    #[account(
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    #[account(
        mut,
        seeds = [KEEPER_SEED, keeper_authority.key().as_ref()],
        bump = keeper.bump,
        constraint = keeper.authority == keeper_authority.key()
            @ LodzError::KeeperAuthorityMismatch,
    )]
    pub keeper: Box<Account<'info, Keeper>>,

    #[account(
        mut,
        seeds = [SEAM_SEED, &seam_id.to_le_bytes()],
        bump = seam.bump,
        constraint = seam.stope_id == stope_id @ LodzError::SeamStopeMismatch,
    )]
    pub seam: Box<Account<'info, Seam>>,

    #[account(
        mut,
        seeds = [STOPE_SEED, &stope_id.to_le_bytes()],
        bump = stope.bump,
    )]
    pub stope: Box<Account<'info, Stope>>,
}

pub fn update_seam_allocation(
    ctx: Context<UpdateSeamAllocation>,
    seam_id: u16,
    stope_id: u8,
    new_allocation_bps: u16,
) -> Result<()> {
    require!(!ctx.accounts.vault_config.paused, LodzError::VaultPaused);
    require!(ctx.accounts.keeper.active, LodzError::KeeperNotActive);
    require!(ctx.accounts.seam.active, LodzError::SeamInactive);
    require!(
        new_allocation_bps <= MAX_BPS,
        LodzError::InvalidAllocationBps
    );

    let now = Clock::get()?.unix_timestamp;

    // Capital cannot be routed into an emission schedule that has already run
    // out. Winding an expired seam down to zero is always allowed.
    if new_allocation_bps > 0 {
        require!(
            ctx.accounts.seam.accrual_window_open(now),
            LodzError::EmissionEnded
        );
    }

    let yield_kind = ctx.accounts.seam.yield_kind;
    let previous_bps = ctx.accounts.seam.allocation_bps;

    // Enforces both the 100 % ceiling and the stope's emissions ceiling.
    ctx.accounts
        .stope
        .reallocate(yield_kind, previous_bps, new_allocation_bps)?;

    let cooldown = ctx.accounts.vault_config.keeper_unbond_cooldown_sec;

    let seam = &mut ctx.accounts.seam;
    seam.allocation_bps = new_allocation_bps;
    seam.last_rebalance_at = now;
    let seam_key = seam.key();

    let stope = &mut ctx.accounts.stope;
    stope.last_rebalance_at = now;
    let stope_allocated_bps = stope.allocated_bps;
    let stope_emissions_bps = stope.emissions_bps;

    let keeper = &mut ctx.accounts.keeper;
    keeper.rebalance_count = keeper
        .rebalance_count
        .checked_add(1)
        .ok_or(LodzError::MathOverflow)?;
    keeper.last_rebalance_at = now;
    keeper.unbond_ready_at = now.checked_add(cooldown).ok_or(LodzError::MathOverflow)?;
    let keeper_key = keeper.key();

    emit!(SeamRebalanced {
        seam: seam_key,
        seam_id,
        stope_id,
        keeper: keeper_key,
        yield_kind,
        previous_allocation_bps: previous_bps,
        new_allocation_bps,
        stope_allocated_bps,
        stope_emissions_bps,
        timestamp: now,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// shared
// ---------------------------------------------------------------------------

/// Keep `VaultConfig::keeper_count` in step with a keeper crossing the minimum
/// bond, so a keeper that crosses it repeatedly is never double counted.
fn apply_active_transition(config: &mut VaultConfig, transition: Option<bool>) -> Result<()> {
    match transition {
        Some(true) => {
            config.keeper_count = config
                .keeper_count
                .checked_add(1)
                .ok_or(LodzError::MathOverflow)?;
        }
        Some(false) => config.keeper_count = config.keeper_count.saturating_sub(1),
        None => {}
    }
    Ok(())
}
