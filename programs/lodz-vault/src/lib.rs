//! LODZ vault -- the on-chain half of a BTC yield layer on Solana.
//!
//! # What this program is
//!
//! Custody and share accounting for tokenized bitcoin deposited through an
//! **Adit**, routing weights for the yield sources (**Seam**) each
//! risk-profiled vault (**Stope**) is exposed to, per-source booking of
//! realized yield, an enforced redemption queue (**Orecart**), and the bond
//! and slash record for the keepers who move allocations.
//!
//! # What this program is not
//!
//! It is not a bridge and it does not hold bitcoin. Every deposit is an SPL
//! Token or Token-2022 token that *represents* bitcoin custodied somewhere
//! else, and `Adit::custody_kind` records which kind of somewhere else. A
//! depositor's exposure includes that issuer and, for a bridged asset, its
//! validator set. Nothing in this program removes that exposure and nothing in
//! it should be read as doing so.
//!
//! It does not execute trades. `Seam::allocation_bps` is the on-chain record
//! of how a stope's capital is *meant* to be routed; the execution against
//! venues happens off-chain in `packages/seam-router`. What the chain enforces
//! is the ceilings (100 % total, and a per-risk-profile ceiling on emissions
//! exposure), the disclosure attached to each seam, and that only a bonded
//! keeper can move a weight.
//!
//! It does not price venue solvency. `accrue_yield` requires the reporting
//! keeper to actually transfer the yield into custody, so the fact that the
//! vault now holds it is checked; the claim about which venue produced it is
//! an assertion by a party with a slashable bond posted, and that is the
//! strongest thing available here. `slash_keeper` is the remedy.
//!
//! # The one thing this program exists to make structural
//!
//! Sustainable yield and emissions yield are never added together. Not on the
//! [`state::Seam`] (a seam is one kind), not on the [`state::Stope`] (two
//! accumulators, two indices), not on the [`state::Miner`] (two balances, and
//! `accrued_yield()` is a view rather than a field), and not in the events an
//! indexer reads. An emissions seam cannot be registered without declaring
//! when its schedule ends, and it stops being able to book yield the moment
//! the chain clock passes that date.
//!
//! # Module map
//!
//! | module         | contents                                        |
//! |----------------|-------------------------------------------------|
//! | `state`        | accounts, PDA seeds, parameter bounds, enums    |
//! | `errors`       | every named failure                             |
//! | `events`       | one event per state transition                  |
//! | `math`         | integer-only helpers, explicit rounding          |
//! | `instructions` | one submodule per instruction group             |
//!
//! # Deployment
//!
//! Nothing in this package deploys anything. There is no deploy script, no CI
//! hook and no cluster other than localnet named anywhere in it. See the
//! README section "Deployment".

use anchor_lang::prelude::*;

pub mod errors;
pub mod events;
pub mod instructions;
pub mod math;
pub mod state;

use instructions::*;
use state::RiskProfile;

declare_id!("F9XmBYVEyEwFyHAdMJs6uBvyRag3AFhQ6YMZvqm13SLW");

#[program]
pub mod lodz_vault {
    use super::*;

    // -- protocol setup ----------------------------------------------------

    /// Create the global config PDA. The vault starts paused: no stope and no
    /// adit exist yet, and a deposit against that state would be accepted
    /// against nothing.
    pub fn initialize_vault(
        ctx: Context<InitializeVault>,
        params: InitializeVaultParams,
    ) -> Result<()> {
        instructions::admin::initialize_vault(ctx, params)
    }

    /// Create `["bond_vault"]`, the $LODZ token account keeper bonds are held
    /// in. Split from `initialize_vault` because `init` on a token account is
    /// stack-expensive.
    pub fn initialize_bond_vault(ctx: Context<InitializeBondVault>) -> Result<()> {
        instructions::admin::initialize_bond_vault(ctx)
    }

    /// Register a BTC representation and create its custody account.
    ///
    /// This is where the disclosure is fixed: the custody kind, the headlamp
    /// risk tier, and the exact ratio at which this token is counted against
    /// the internal accounting unit. A mint with no adit cannot be deposited.
    pub fn register_adit(ctx: Context<RegisterAdit>, params: RegisterAditParams) -> Result<()> {
        instructions::admin::register_adit(ctx, params)
    }

    /// Open one of the three stopes together with its redemption queue.
    ///
    /// `risk_profile` must be the canonical profile for `stope_id`
    /// (0 conservative, 1 balanced, 2 aggressive); the program rejects any
    /// other pairing so the id is a reliable key off-chain.
    pub fn open_stope(
        ctx: Context<OpenStope>,
        stope_id: u8,
        risk_profile: RiskProfile,
    ) -> Result<()> {
        instructions::admin::open_stope(ctx, stope_id, risk_profile)
    }

    /// Register a yield source against a stope.
    ///
    /// An emissions seam must declare a future `emission_ends_at` and the mint
    /// the emission is paid in. A sustainable seam must leave both unset. The
    /// seam's risk tier and the stope's resulting emissions exposure are both
    /// checked against the stope's risk profile.
    pub fn register_seam(
        ctx: Context<RegisterSeam>,
        seam_id: u16,
        stope_id: u8,
        params: RegisterSeamParams,
    ) -> Result<()> {
        instructions::admin::register_seam(ctx, seam_id, stope_id, params)
    }

    // -- circuit breaker ---------------------------------------------------

    /// Halt deposits, accrual, redemption requests and rebalances.
    /// `claim_redemption` keeps running.
    pub fn pause_vault(ctx: Context<SetPause>) -> Result<()> {
        instructions::admin::pause_vault(ctx)
    }

    /// Resume.
    pub fn unpause_vault(ctx: Context<SetPause>) -> Result<()> {
        instructions::admin::unpause_vault(ctx)
    }

    // -- authority handover ------------------------------------------------

    /// Nominate the next authority. Takes effect only once it signs
    /// `accept_authority`.
    pub fn propose_authority(ctx: Context<ProposeAuthority>) -> Result<()> {
        instructions::admin::propose_authority(ctx)
    }

    /// Accept a pending authority handover.
    pub fn accept_authority(ctx: Context<AcceptAuthority>) -> Result<()> {
        instructions::admin::accept_authority(ctx)
    }

    // -- depositors --------------------------------------------------------

    /// Deposit a registered BTC representation into a stope and receive
    /// shares. Creates the caller's `Miner` position on first use.
    pub fn deposit(ctx: Context<MakeDeposit>, stope_id: u8, amount: u64) -> Result<()> {
        instructions::deposit::deposit(ctx, stope_id, amount)
    }

    /// Burn shares and issue an Orecart ticket.
    ///
    /// `ticket_index` must equal the caller's current `Miner::ticket_count`;
    /// it is the third PDA seed of the ticket. The fee and the wait are
    /// computed and frozen onto the ticket here.
    pub fn request_redemption(
        ctx: Context<RequestRedemption>,
        stope_id: u8,
        ticket_index: u32,
        shares: u64,
    ) -> Result<()> {
        instructions::redemption::request_redemption(ctx, stope_id, ticket_index, shares)
    }

    /// Pay out a ticket whose `claimable_at` has passed. Refuses before it.
    pub fn claim_redemption(
        ctx: Context<ClaimRedemption>,
        stope_id: u8,
        ticket_index: u32,
    ) -> Result<()> {
        instructions::redemption::claim_redemption(ctx, stope_id, ticket_index)
    }

    // -- keepers -----------------------------------------------------------

    /// Post or increase a $LODZ bond. Creates the caller's `Keeper` record on
    /// first use.
    pub fn bond_keeper(ctx: Context<BondKeeper>, amount: u64) -> Result<()> {
        instructions::keeper::bond_keeper(ctx, amount)
    }

    /// Withdraw bond, no earlier than `keeper_unbond_cooldown_sec` after the
    /// keeper's last rebalance.
    pub fn unbond_keeper(ctx: Context<UnbondKeeper>, amount: u64) -> Result<()> {
        instructions::keeper::unbond_keeper(ctx, amount)
    }

    /// Move a seam's allocation. Requires an active bonded keeper, and
    /// re-checks the stope's 100 % ceiling and its emissions ceiling.
    pub fn update_seam_allocation(
        ctx: Context<UpdateSeamAllocation>,
        seam_id: u16,
        stope_id: u8,
        new_allocation_bps: u16,
    ) -> Result<()> {
        instructions::keeper::update_seam_allocation(ctx, seam_id, stope_id, new_allocation_bps)
    }

    /// Book realized yield against one seam.
    ///
    /// The reporting keeper transfers `amount` of the seam's asset into
    /// custody; the program attributes it to that seam's declared
    /// [`state::YieldKind`] and to no other. Requires an active bond, and refuses
    /// once an emissions seam's declared schedule has ended.
    pub fn accrue_yield(
        ctx: Context<AccrueYield>,
        seam_id: u16,
        stope_id: u8,
        amount: u64,
    ) -> Result<()> {
        instructions::accrual::accrue_yield(ctx, seam_id, stope_id, amount)
    }

    // -- enforcement -------------------------------------------------------

    /// Take a keeper's bond. Authority only; the taken amount moves to the
    /// treasury recorded at initialization, and the keeper deactivates
    /// automatically if the slash drops it below the minimum bond.
    pub fn slash_keeper(ctx: Context<SlashKeeper>, amount: u64, reason_code: u16) -> Result<()> {
        instructions::admin::slash_keeper(ctx, amount, reason_code)
    }
}
