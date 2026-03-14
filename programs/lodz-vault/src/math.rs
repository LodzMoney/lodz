//! Integer-only arithmetic helpers.
//!
//! Floating point is unavailable on SBF and non-deterministic across
//! validators, so every ratio in this program is basis points (1/10_000) and
//! every yield accumulator is a fixed-point `u128` scaled by
//! [`YIELD_INDEX_SCALE`].
//!
//! # Rounding
//!
//! Every division here floors. The direction is chosen so that rounding dust
//! stays in the vault rather than leaving it:
//!
//! - deposit normalization floors, so a depositor is credited at most what
//!   they paid in;
//! - redemption denormalization floors, so a ticket pays out at most what the
//!   burned shares are worth;
//! - the yield index floors, so the sum of what miners can claim is never more
//!   than what was accrued.
//!
//! The residue is a few base units per operation and it is not swept anywhere:
//! it simply raises the vault's backing.

use anchor_lang::prelude::*;

use crate::errors::LodzError;

/// Basis-point denominator. 10_000 bps == 100 %.
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Largest meaningful basis-point value.
pub const MAX_BPS: u16 = 10_000;

/// Fixed-point scale of the per-share yield accumulators (`1e12`).
///
/// A stope's index is `sum(accrued_amount * SCALE / total_shares)`. With
/// shares denominated in satoshi-equivalent units, `1e12` keeps a single
/// satoshi of yield distributable across a stope holding up to `1e12` shares
/// (10_000 BTC) without truncating to zero.
pub const YIELD_INDEX_SCALE: u128 = 1_000_000_000_000;

/// Seconds in a day, used by the queue congestion model.
pub const SECONDS_PER_DAY: i64 = 86_400;

/// `floor(a * b / denominator)` computed in `u128`, narrowed back to `u64`.
pub fn mul_div_floor(a: u64, b: u64, denominator: u64) -> Result<u64> {
    require!(denominator != 0, LodzError::DivisionByZero);

    let product = (a as u128)
        .checked_mul(b as u128)
        .ok_or(LodzError::MathOverflow)?;
    let quotient = product / (denominator as u128);

    u64::try_from(quotient).map_err(|_| error!(LodzError::MathOverflow))
}

/// `floor(amount * bps / 10_000)`.
pub fn bps_fraction(amount: u64, bps: u16) -> Result<u64> {
    mul_div_floor(amount, bps as u64, BPS_DENOMINATOR)
}

/// Convert a deposit denominated in an asset's native units into the vault's
/// internal accounting unit.
///
/// `num` / `den` are pinned per Adit at registration and fold together two
/// separate things: the decimal difference between the asset mint and the
/// internal 8-decimal unit, and the asset's declared ratio to one BTC.
pub fn to_normalized(amount: u64, num: u64, den: u64) -> Result<u64> {
    require!(num != 0 && den != 0, LodzError::InvalidConversionRatio);
    mul_div_floor(amount, num, den)
}

/// Inverse of [`to_normalized`]. Also floors, so a round trip can lose up to
/// one base unit in each direction. That dust stays in the vault.
pub fn from_normalized(normalized: u64, num: u64, den: u64) -> Result<u64> {
    require!(num != 0 && den != 0, LodzError::InvalidConversionRatio);
    mul_div_floor(normalized, den, num)
}

/// Per-share index increment for `amount` of realized yield spread over
/// `total_shares`.
///
/// Errors rather than returning zero when the stope holds no shares: silently
/// dropping an accrual would make the reported per-source totals disagree with
/// what miners can actually claim, and that disagreement is exactly what this
/// program exists to prevent.
pub fn index_delta(amount: u64, total_shares: u64) -> Result<u128> {
    require!(total_shares > 0, LodzError::NoSharesOutstanding);

    let scaled = (amount as u128)
        .checked_mul(YIELD_INDEX_SCALE)
        .ok_or(LodzError::MathOverflow)?;

    Ok(scaled / (total_shares as u128))
}

/// Yield owed to a holder of `shares` between an index snapshot and the
/// current index.
pub fn pending_from_index(shares: u64, index_now: u128, index_snapshot: u128) -> Result<u64> {
    // saturating: an index can only ever move up, but a snapshot taken from a
    // future upgrade must not be able to underflow this into a huge payout.
    let delta = index_now.saturating_sub(index_snapshot);
    if delta == 0 || shares == 0 {
        return Ok(0);
    }

    let scaled = (shares as u128)
        .checked_mul(delta)
        .ok_or(LodzError::MathOverflow)?;

    u64::try_from(scaled / YIELD_INDEX_SCALE).map_err(|_| error!(LodzError::MathOverflow))
}

/// Extra seconds a redemption ticket must wait because of the queue already
/// ahead of it.
///
/// `pending_ahead` is in internal accounting units, `drain_per_day` is how
/// many of those units the protocol commits to being able to settle per day.
/// The result is `floor(pending_ahead * 86_400 / drain_per_day)`: a queue that
/// already holds one full day of drain capacity adds one day of wait.
///
/// This is the whole point of the Orecart being on-chain. The delay is not a
/// number a front end displays, it is a number `claim_redemption` refuses to
/// let a caller past.
pub fn queue_delay_sec(pending_ahead: u64, drain_per_day: u64) -> Result<i64> {
    require!(drain_per_day != 0, LodzError::InvalidQueueDrain);

    let scaled = (pending_ahead as u128)
        .checked_mul(SECONDS_PER_DAY as u128)
        .ok_or(LodzError::MathOverflow)?;
    let seconds = scaled / (drain_per_day as u128);

    i64::try_from(seconds).map_err(|_| error!(LodzError::MathOverflow))
}

/// Validate a fixed-width NUL-padded ASCII label.
///
/// Rejects an all-zero name and any byte outside printable ASCII, so an
/// indexer never has to guess at the encoding of a venue string it renders.
pub fn validate_ascii_label(bytes: &[u8]) -> bool {
    let mut seen_nul = false;
    let mut non_empty = false;

    for &b in bytes {
        if b == 0 {
            seen_nul = true;
            continue;
        }
        // Once padding starts it must not stop.
        if seen_nul {
            return false;
        }
        if !(0x20..=0x7e).contains(&b) {
            return false;
        }
        non_empty = true;
    }

    non_empty
}
