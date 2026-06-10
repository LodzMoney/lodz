//! Orecart -- `request_redemption` and `claim_redemption`.
//!
//! # The queue is the product, so the queue is on-chain
//!
//! Every BTC yield product has a redemption delay. Most of them keep it in a
//! terms-of-service page, where it can be changed after the fact and where
//! nothing stops the operator from letting one depositor out ahead of another.
//! Here the wait is computed when the ticket is issued, written into the
//! ticket account, and enforced by `claim_redemption` against the chain clock.
//! The fee is captured on the ticket at the same moment, so a fee change by
//! the authority cannot reach a request that is already queued.
//!
//! # Shares burn on request, not on claim
//!
//! Requesting a redemption removes the shares from the stope immediately. The
//! consequence is deliberate: a depositor in the queue stops earning yield the
//! moment they queue, because they are no longer taking the risk that produces
//! it. It also removes any question of a queued position being double counted.
//!
//! # What the delay is computed from
//!
//! `claimable_at = now + base_delay + floor(backlog * 86400 / drain_per_day)`,
//! clamped to `max_redemption_delay_sec`. The backlog term is the queue that
//! was already standing there when the ticket was issued, so a depositor
//! joining a congested queue is told a longer wait than one joining an empty
//! one, and both are held to what they were told.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::errors::LodzError;
use crate::events::{RedemptionClaimed, RedemptionRequested};
use crate::math::{bps_fraction, from_normalized, mul_div_floor, queue_delay_sec};
use crate::state::*;

// ---------------------------------------------------------------------------
// request_redemption
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(stope_id: u8, ticket_index: u32)]
pub struct RequestRedemption<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    #[account(
        mut,
        seeds = [STOPE_SEED, &stope_id.to_le_bytes()],
        bump = stope.bump,
    )]
    pub stope: Box<Account<'info, Stope>>,

    #[account(
        mut,
        seeds = [MINER_SEED, owner.key().as_ref(), &stope_id.to_le_bytes()],
        bump = miner.bump,
        has_one = owner @ LodzError::Unauthorized,
    )]
    pub miner: Box<Account<'info, Miner>>,

    #[account(
        mut,
        seeds = [ORECART_QUEUE_SEED, &stope_id.to_le_bytes()],
        bump = orecart_queue.bump,
        constraint = orecart_queue.stope_id == stope_id @ LodzError::QueueStopeMismatch,
    )]
    pub orecart_queue: Box<Account<'info, OrecartQueue>>,

    #[account(
        init,
        payer = owner,
        space = 8 + Orecart::LEN,
        seeds = [ORECART_SEED, owner.key().as_ref(), &ticket_index.to_le_bytes()],
        bump
    )]
    pub orecart: Box<Account<'info, Orecart>>,

    /// The asset this ticket will pay out in. It does not have to be the asset
    /// the depositor put in: the vault keeps its books in internal accounting
    /// units, so any registered adit can settle a ticket, subject to that
    /// asset's custody account holding enough at claim time.
    #[account(
        seeds = [ADIT_SEED, asset_mint.key().as_ref()],
        bump = adit.bump,
        has_one = asset_mint @ LodzError::AditMintMismatch,
    )]
    pub adit: Box<Account<'info, Adit>>,

    pub asset_mint: Box<InterfaceAccount<'info, Mint>>,

    pub system_program: Program<'info, System>,
}

pub fn request_redemption(
    ctx: Context<RequestRedemption>,
    stope_id: u8,
    ticket_index: u32,
    shares: u64,
) -> Result<()> {
    require!(!ctx.accounts.vault_config.paused, LodzError::VaultPaused);
    require!(!ctx.accounts.stope.paused, LodzError::StopePaused);
    require!(shares > 0, LodzError::ZeroAmount);
    require!(
        ticket_index == ctx.accounts.miner.ticket_count,
        LodzError::TicketIndexMismatch
    );
    require!(
        ctx.accounts.stope.total_shares > 0,
        LodzError::NoSharesOutstanding
    );

    let now = Clock::get()?.unix_timestamp;

    // Bank the position's yield attribution before the share count changes.
    let index_sustainable = ctx.accounts.stope.yield_index_sustainable;
    let index_emissions = ctx.accounts.stope.yield_index_emissions;
    ctx.accounts
        .miner
        .settle_indices(index_sustainable, index_emissions)?;

    let shares_before = ctx.accounts.miner.shares;
    require!(shares <= shares_before, LodzError::InsufficientShares);

    // Principal owed, at the pool's current ratio. Because `accrue_yield`
    // raises `total_deposits` without raising `total_shares`, this is
    // principal plus the position's realized yield in one number.
    let normalized = mul_div_floor(
        shares,
        ctx.accounts.stope.total_deposits,
        ctx.accounts.stope.total_shares,
    )?;
    require!(normalized > 0, LodzError::ZeroPayout);

    let fee_bps = ctx.accounts.vault_config.fee_bps;
    let fee_normalized = bps_fraction(normalized, fee_bps)?;
    let payout_normalized = normalized.saturating_sub(fee_normalized);

    let conversion_num = ctx.accounts.adit.conversion_num;
    let conversion_den = ctx.accounts.adit.conversion_den;
    let gross_amount = from_normalized(normalized, conversion_num, conversion_den)?;
    let payout_amount = from_normalized(payout_normalized, conversion_num, conversion_den)?;
    let fee_amount = gross_amount.saturating_sub(payout_amount);
    require!(payout_amount > 0, LodzError::ZeroPayout);

    // The wait, computed against the backlog that is already there.
    let pending_ahead = ctx.accounts.orecart_queue.total_pending;
    let congestion = queue_delay_sec(pending_ahead, ctx.accounts.vault_config.queue_drain_per_day)?;
    let total_delay = ctx
        .accounts
        .vault_config
        .redemption_delay_sec
        .checked_add(congestion)
        .ok_or(LodzError::MathOverflow)?
        .min(ctx.accounts.vault_config.max_redemption_delay_sec);
    let claimable_at = now.checked_add(total_delay).ok_or(LodzError::MathOverflow)?;

    // Move the position's yield attribution from "still held" to "taken out",
    // in proportion to the shares being redeemed, keeping the two sources
    // apart on the way through.
    let claimed_sustainable = mul_div_floor(
        ctx.accounts.miner.accrued_sustainable,
        shares,
        shares_before,
    )?;
    let claimed_emissions =
        mul_div_floor(ctx.accounts.miner.accrued_emissions, shares, shares_before)?;

    let owner_key = ctx.accounts.owner.key();
    let queue_position = ctx.accounts.orecart_queue.tail;

    let miner = &mut ctx.accounts.miner;
    miner.shares = miner.shares.saturating_sub(shares);
    miner.pending_redemption = miner
        .pending_redemption
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    miner.ticket_count = miner
        .ticket_count
        .checked_add(1)
        .ok_or(LodzError::MathOverflow)?;
    miner.accrued_sustainable = miner.accrued_sustainable.saturating_sub(claimed_sustainable);
    miner.accrued_emissions = miner.accrued_emissions.saturating_sub(claimed_emissions);
    miner.claimed_sustainable = miner
        .claimed_sustainable
        .checked_add(claimed_sustainable)
        .ok_or(LodzError::MathOverflow)?;
    miner.claimed_emissions = miner
        .claimed_emissions
        .checked_add(claimed_emissions)
        .ok_or(LodzError::MathOverflow)?;
    miner.last_action_at = now;

    let stope = &mut ctx.accounts.stope;
    stope.total_shares = stope.total_shares.saturating_sub(shares);
    stope.total_deposits = stope.total_deposits.saturating_sub(normalized);
    stope.pending_redemption = stope
        .pending_redemption
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;

    let queue = &mut ctx.accounts.orecart_queue;
    queue.tail = queue.tail.checked_add(1).ok_or(LodzError::MathOverflow)?;
    queue.pending_tickets = queue
        .pending_tickets
        .checked_add(1)
        .ok_or(LodzError::MathOverflow)?;
    queue.total_pending = queue
        .total_pending
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    queue.last_request_at = now;

    let config = &mut ctx.accounts.vault_config;
    config.total_normalized_deposits = config.total_normalized_deposits.saturating_sub(normalized);

    let asset_mint = ctx.accounts.asset_mint.key();

    let ticket = &mut ctx.accounts.orecart;
    ticket.owner = owner_key;
    ticket.asset_mint = asset_mint;
    ticket.ticket_index = ticket_index;
    ticket.stope_id = stope_id;
    ticket.status = TicketStatus::Queued;
    ticket.bump = ctx.bumps.orecart;
    ticket.fee_bps = fee_bps;
    ticket.shares_burned = shares;
    ticket.normalized_amount = normalized;
    ticket.fee_normalized = fee_normalized;
    ticket.gross_amount = gross_amount;
    ticket.fee_amount = fee_amount;
    ticket.payout_amount = payout_amount;
    ticket.queue_position = queue_position;
    ticket.requested_at = now;
    ticket.claimable_at = claimable_at;
    ticket.claimed_at = 0;

    emit!(RedemptionRequested {
        owner: owner_key,
        orecart: ticket.key(),
        ticket_index,
        stope_id,
        asset_mint,
        shares_burned: shares,
        normalized_amount: normalized,
        fee_bps,
        fee_normalized,
        gross_amount,
        payout_amount,
        queue_position,
        queue_pending_ahead: pending_ahead,
        requested_at: now,
        claimable_at,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// claim_redemption
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(stope_id: u8, ticket_index: u32)]
pub struct ClaimRedemption<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [VAULT_CONFIG_SEED],
        bump = vault_config.bump,
    )]
    pub vault_config: Box<Account<'info, VaultConfig>>,

    #[account(
        mut,
        seeds = [STOPE_SEED, &stope_id.to_le_bytes()],
        bump = stope.bump,
    )]
    pub stope: Box<Account<'info, Stope>>,

    #[account(
        mut,
        seeds = [MINER_SEED, owner.key().as_ref(), &stope_id.to_le_bytes()],
        bump = miner.bump,
        has_one = owner @ LodzError::Unauthorized,
    )]
    pub miner: Box<Account<'info, Miner>>,

    #[account(
        mut,
        seeds = [ORECART_SEED, owner.key().as_ref(), &ticket_index.to_le_bytes()],
        bump = orecart.bump,
        has_one = owner @ LodzError::Unauthorized,
        has_one = asset_mint @ LodzError::AditMintMismatch,
        constraint = orecart.stope_id == stope_id @ LodzError::TicketStopeMismatch,
    )]
    pub orecart: Box<Account<'info, Orecart>>,

    #[account(
        mut,
        seeds = [ORECART_QUEUE_SEED, &stope_id.to_le_bytes()],
        bump = orecart_queue.bump,
        constraint = orecart_queue.stope_id == stope_id @ LodzError::QueueStopeMismatch,
    )]
    pub orecart_queue: Box<Account<'info, OrecartQueue>>,

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
        seeds = [ADIT_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        token::mint = asset_mint,
        token::authority = vault_config,
        token::token_program = token_program,
    )]
    pub adit_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = asset_mint,
        token::authority = owner,
        token::token_program = token_program,
    )]
    pub owner_token: Box<InterfaceAccount<'info, TokenAccount>>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Pay out a queued ticket.
///
/// Deliberately not gated on `VaultConfig::paused`. A ticket whose delay has
/// elapsed is a settled debt, and a circuit breaker that can strand it would
/// make every quoted wait conditional on the authority's goodwill.
pub fn claim_redemption(
    ctx: Context<ClaimRedemption>,
    stope_id: u8,
    ticket_index: u32,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    require!(
        ctx.accounts.orecart.status == TicketStatus::Queued,
        LodzError::TicketAlreadyClaimed
    );
    // The gate. Everything else in the Orecart design exists to make this line
    // meaningful.
    require!(
        now >= ctx.accounts.orecart.claimable_at,
        LodzError::RedemptionNotClaimable
    );

    let payout_amount = ctx.accounts.orecart.payout_amount;
    let fee_amount = ctx.accounts.orecart.fee_amount;
    let normalized = ctx.accounts.orecart.normalized_amount;
    let fee_normalized = ctx.accounts.orecart.fee_normalized;
    let requested_at = ctx.accounts.orecart.requested_at;

    require!(payout_amount > 0, LodzError::ZeroPayout);
    require!(
        ctx.accounts.adit_vault.amount >= payout_amount,
        LodzError::InsufficientVaultLiquidity
    );

    let decimals = ctx.accounts.adit.decimals;
    let config_bump = [ctx.accounts.vault_config.bump];
    let seeds = VaultConfig::signer_seeds(&config_bump);
    let signer_seeds: &[&[&[u8]]] = &[&seeds];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.adit_vault.to_account_info(),
                mint: ctx.accounts.asset_mint.to_account_info(),
                to: ctx.accounts.owner_token.to_account_info(),
                authority: ctx.accounts.vault_config.to_account_info(),
            },
            signer_seeds,
        ),
        payout_amount,
        decimals,
    )?;

    let ticket = &mut ctx.accounts.orecart;
    ticket.status = TicketStatus::Claimed;
    ticket.claimed_at = now;
    let ticket_key = ticket.key();

    let miner = &mut ctx.accounts.miner;
    miner.pending_redemption = miner.pending_redemption.saturating_sub(normalized);
    miner.withdrawn = miner
        .withdrawn
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    miner.last_action_at = now;

    let stope = &mut ctx.accounts.stope;
    stope.pending_redemption = stope.pending_redemption.saturating_sub(normalized);
    stope.total_redeemed = stope
        .total_redeemed
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;

    let queue = &mut ctx.accounts.orecart_queue;
    queue.head = queue.head.checked_add(1).ok_or(LodzError::MathOverflow)?;
    queue.pending_tickets = queue.pending_tickets.saturating_sub(1);
    queue.total_pending = queue.total_pending.saturating_sub(normalized);
    queue.total_claimed = queue
        .total_claimed
        .checked_add(normalized)
        .ok_or(LodzError::MathOverflow)?;
    queue.total_fees = queue
        .total_fees
        .checked_add(fee_normalized)
        .ok_or(LodzError::MathOverflow)?;
    queue.last_claim_at = now;
    let queue_total_pending = queue.total_pending;

    // Only the payout leaves custody. The fee stays in the adit vault as
    // protocol surplus, and it is deliberately not added back into
    // `Stope::total_deposits`: it does not raise any share's accounted value,
    // it sits as backing in excess of what the books claim. `total_fees` on
    // the queue is the running record of it.
    let adit = &mut ctx.accounts.adit;
    adit.total_deposited = adit.total_deposited.saturating_sub(payout_amount);

    emit!(RedemptionClaimed {
        owner: ctx.accounts.owner.key(),
        orecart: ticket_key,
        ticket_index,
        stope_id,
        asset_mint: ctx.accounts.asset_mint.key(),
        payout_amount,
        fee_amount,
        normalized_amount: normalized,
        waited_sec: now.saturating_sub(requested_at),
        queue_total_pending,
        claimed_at: now,
    });

    Ok(())
}
