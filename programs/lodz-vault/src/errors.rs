//! Every failure mode this program can produce has a named error.
//!
//! There is no `unwrap()`, `expect()` or `panic!()` anywhere under
//! `programs/`. A panic inside an SBF program aborts with a generic code that
//! carries no information to the caller and none to an indexer, so every
//! fallible path here goes through `require!`, `?` or an explicit `ok_or`.

use anchor_lang::prelude::*;

#[error_code]
pub enum LodzError {
    // -- authority ---------------------------------------------------------
    #[msg("Signer is not the vault authority.")]
    Unauthorized,
    #[msg("No authority transfer is pending.")]
    NoPendingAuthority,
    #[msg("Signer is not the pending authority.")]
    NotPendingAuthority,

    // -- lifecycle ---------------------------------------------------------
    #[msg("The vault is paused.")]
    VaultPaused,
    #[msg("The vault is not paused.")]
    VaultNotPaused,
    #[msg("This stope is paused.")]
    StopePaused,
    #[msg("This adit is paused.")]
    AditPaused,

    // -- arithmetic --------------------------------------------------------
    #[msg("Arithmetic overflow.")]
    MathOverflow,
    #[msg("Division by zero.")]
    DivisionByZero,
    #[msg("Amount must be greater than zero.")]
    ZeroAmount,

    // -- configuration bounds ---------------------------------------------
    #[msg("Fee exceeds the hard cap enforced by the program.")]
    InvalidFeeBps,
    #[msg("Redemption delay is outside the bounds enforced by the program.")]
    InvalidDelay,
    #[msg("Queue drain rate must be greater than zero.")]
    InvalidQueueDrain,
    #[msg("Minimum keeper bond must be greater than zero.")]
    InvalidMinKeeperBond,
    #[msg("Conversion ratio numerator and denominator must both be non-zero.")]
    InvalidConversionRatio,
    #[msg("Mint decimals exceed the supported range.")]
    InvalidDecimals,
    #[msg("Label is empty or not NUL-padded ASCII.")]
    InvalidLabel,
    #[msg("Venue name is empty or not NUL-padded ASCII.")]
    InvalidVenueName,
    #[msg("Risk tier is outside the 1..=5 headlamp range.")]
    InvalidRiskTier,

    // -- stope -------------------------------------------------------------
    #[msg("Stope id must be 0 (conservative), 1 (balanced) or 2 (aggressive).")]
    InvalidStopeId,
    #[msg("Risk profile does not match the canonical profile for this stope id.")]
    RiskProfileMismatch,
    #[msg("This seam's risk tier is above what this stope's risk profile permits.")]
    RiskTierExceedsStopeProfile,
    #[msg("The stope holds no shares, so there is nothing to distribute yield to.")]
    NoSharesOutstanding,

    // -- seam --------------------------------------------------------------
    #[msg("Allocation basis points exceed 10000.")]
    InvalidAllocationBps,
    #[msg("Total seam allocation for this stope would exceed 10000 basis points.")]
    AllocationExceeded,
    #[msg("Emissions-backed seams must declare a future emission_ends_at.")]
    EmissionEndMissing,
    #[msg("emission_ends_at is in the past.")]
    EmissionEndInPast,
    #[msg("A sustainable seam must leave emission_ends_at and emission_mint unset.")]
    EmissionFieldsOnSustainableSeam,
    #[msg("Emissions-backed seams must name the mint the emission is paid in.")]
    EmissionMintMissing,
    #[msg("This seam's emission window has ended; it can no longer accrue yield.")]
    EmissionEnded,
    #[msg("Emissions allocation would exceed what this stope's risk profile permits.")]
    EmissionsAllocationExceeded,
    #[msg("The seam is not active.")]
    SeamInactive,
    #[msg("The seam does not belong to the supplied stope.")]
    SeamStopeMismatch,
    #[msg("The seam's asset mint does not match the supplied adit.")]
    SeamAssetMismatch,

    // -- adit / deposits ---------------------------------------------------
    #[msg("The adit does not accept this mint.")]
    AditMintMismatch,
    #[msg("The supplied vault is not the vault recorded on this adit.")]
    AditVaultMismatch,
    #[msg("Deposit would exceed this adit's cap.")]
    DepositCapExceeded,
    #[msg("Deposit is too small to credit a single accounting unit.")]
    DepositBelowMinimum,
    #[msg("The token program does not match the one pinned at registration.")]
    TokenProgramMismatch,

    // -- redemption / orecart ---------------------------------------------
    #[msg("Requested shares exceed the miner's balance.")]
    InsufficientShares,
    #[msg("Ticket index does not match the miner's next ticket index.")]
    TicketIndexMismatch,
    #[msg("This ticket has already been claimed.")]
    TicketAlreadyClaimed,
    #[msg("The redemption delay has not elapsed yet.")]
    RedemptionNotClaimable,
    #[msg("The queue does not belong to the supplied stope.")]
    QueueStopeMismatch,
    #[msg("The ticket does not belong to the supplied stope.")]
    TicketStopeMismatch,
    #[msg("Redemption resolves to a zero payout after fees.")]
    ZeroPayout,
    #[msg("The adit vault does not hold enough of this asset to settle the ticket.")]
    InsufficientVaultLiquidity,

    // -- keeper ------------------------------------------------------------
    #[msg("The keeper is not active.")]
    KeeperNotActive,
    #[msg("Bond is below the minimum required to be an active keeper.")]
    KeeperBondTooSmall,
    #[msg("The keeper record does not belong to the supplied authority.")]
    KeeperAuthorityMismatch,
    #[msg("The keeper unbond cooldown has not elapsed since its last rebalance.")]
    KeeperUnbondCooldown,
    #[msg("Requested amount exceeds the keeper's bonded balance.")]
    InsufficientBond,
    #[msg("The supplied treasury does not match the one recorded in the config.")]
    TreasuryMismatch,
    // Appended rather than filed beside EmissionsAllocationExceeded: an Anchor
    // error code is its position in this enum, and inserting in the middle
    // renumbers every variant after it. Client code and this document's
    // recorded control-group codes would silently start meaning other things.
    #[msg("Counterparty allocation would exceed what this stope's risk profile permits.")]
    CounterpartyAllocationExceeded,
}
