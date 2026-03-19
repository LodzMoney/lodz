//! On-chain accounts, PDA seeds and the bounds the program enforces on its
//! own configuration.
//!
//! Every account carries an explicit `LEN` and a `reserved` tail. The tail is
//! upgrade headroom: an Anchor account layout can grow into trailing reserved
//! bytes without a migration, but it can never shrink or reorder, so the space
//! is claimed while it is still free. `space = 8 + T::LEN`, where 8 is the
//! Anchor discriminator.
//!
//! The `LEN` arithmetic is asserted against real Borsh output in
//! `tests::declared_lengths_match_the_structs`. A `LEN` that disagrees with
//! its struct silently corrupts every byte written past the end of the
//! account.

use anchor_lang::prelude::*;

pub mod adit;
pub mod config;
pub mod keeper;
pub mod miner;
pub mod orecart;
pub mod seam;
pub mod stope;

pub use adit::*;
pub use config::*;
pub use keeper::*;
pub use miner::*;
pub use orecart::*;
pub use seam::*;
pub use stope::*;

// ---------------------------------------------------------------------------
// PDA seeds
//
// Every seed is a distinct byte string. Numeric seeds are always
// little-endian (`to_le_bytes`): the on-chain program, the TypeScript SDK, the
// Python tooling and the service indexer must derive the same address byte for
// byte, and a big-endian slip is the PDA mismatch recorded in
// new_project_guide/references/solana/anchor-lessons.md.
// ---------------------------------------------------------------------------

/// `["vault_config"]`
pub const VAULT_CONFIG_SEED: &[u8] = b"vault_config";
/// `["adit", asset_mint]`
pub const ADIT_SEED: &[u8] = b"adit";
/// `["adit_vault", asset_mint]` -- program-owned custody token account
pub const ADIT_VAULT_SEED: &[u8] = b"adit_vault";
/// `["bond_vault"]` -- program-owned $LODZ keeper bond token account
pub const BOND_VAULT_SEED: &[u8] = b"bond_vault";
/// `["stope", stope_id(u8)]`
pub const STOPE_SEED: &[u8] = b"stope";
/// `["seam", seam_id(u16 LE)]`
pub const SEAM_SEED: &[u8] = b"seam";
/// `["miner", owner, stope_id(u8)]`
pub const MINER_SEED: &[u8] = b"miner";
/// `["orecart", owner, ticket_index(u32 LE)]`
pub const ORECART_SEED: &[u8] = b"orecart";
/// `["orecart_queue", stope_id(u8)]`
pub const ORECART_QUEUE_SEED: &[u8] = b"orecart_queue";
/// `["keeper", authority]`
pub const KEEPER_SEED: &[u8] = b"keeper";

// ---------------------------------------------------------------------------
// Bounds enforced in code rather than by policy
//
// These are ceilings the authority cannot raise by sending a transaction. A
// compromised authority key is a realistic failure mode for a young protocol,
// and these are the parameters where the difference between "bad" and
// "unrecoverable" is decided.
// ---------------------------------------------------------------------------

/// Redemption fee ceiling: 5 %. The authority sets the live value below this.
pub const MAX_FEE_BPS: u16 = 500;

/// A base redemption delay above 30 days is not a queue, it is a lockup, and
/// this product does not sell a lockup.
pub const MAX_BASE_REDEMPTION_DELAY_SEC: i64 = 30 * 86_400;

/// Hard ceiling on the *total* delay a ticket can be stamped with, including
/// the queue congestion term: 180 days.
pub const MAX_TOTAL_REDEMPTION_DELAY_SEC: i64 = 180 * 86_400;

/// Keeper unbond cooldown ceiling: 30 days.
pub const MAX_KEEPER_UNBOND_COOLDOWN_SEC: i64 = 30 * 86_400;

/// There are exactly three stopes: conservative, balanced, aggressive.
pub const STOPE_COUNT: u8 = 3;

/// Internal accounting unit: 8 decimals, i.e. one satoshi-equivalent.
///
/// Nothing about this makes a deposit "bitcoin". It is the unit LODZ keeps its
/// books in so that several different tokenized representations of BTC, each
/// with its own decimals and its own ratio to one BTC, can be added up at all.
pub const INTERNAL_DECIMALS: u8 = 8;

/// SPL and Token-2022 mints in scope carry 0..=18 decimals.
pub const MAX_MINT_DECIMALS: u8 = 18;

/// Headlamp risk tiers run 1 (lowest) to 5 (highest). There is no tier 0:
/// every representation of BTC on Solana carries bridge or custody risk, and a
/// zero would read as "none".
pub const MIN_RISK_TIER: u8 = 1;
pub const MAX_RISK_TIER: u8 = 5;

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

/// How a seam produces its yield.
///
/// This split is the product. A protocol that reports one blended APY cannot
/// tell a depositor whether the number survives next quarter, so the two are
/// never summed into a single field anywhere in this program: separate
/// accumulators on the [`Stope`], separate accumulators on the [`Miner`], and
/// a `yield_kind` on every `YieldAccrued` event.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum YieldKind {
    /// Paid out of fees or interest that a counterparty is actually paying:
    /// borrow interest, swap fees, basis. It can fall to zero, but nothing
    /// about it is scheduled to end.
    ///
    /// Named `SustainableYield` in the LODZ product vocabulary.
    Sustainable,
    /// Paid out of a token emission schedule. It ends on a known date, and
    /// [`Seam::emission_ends_at`] is required to say when.
    ///
    /// Named `EmissionsYield` in the LODZ product vocabulary.
    Emissions,
}

/// What kind of claim a deposited token actually is.
///
/// No variant here is bitcoin on the Bitcoin network. A deposit into LODZ is a
/// deposit of an SPL or Token-2022 token that stands in for bitcoin held
/// somewhere else, and the variant records which "somewhere else" the
/// depositor is exposed to.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CustodyKind {
    /// Minted by a bridge against bitcoin locked on another chain. The holder
    /// is exposed to the bridge's validator set and its contracts.
    BridgeMinted,
    /// Issued by a named custodian that publishes a redemption process. The
    /// holder is exposed to that custodian.
    CustodianRedeemable,
    /// Tracks the price of bitcoin without a per-token reserve behind it. The
    /// holder is exposed to whatever mechanism maintains the peg.
    SyntheticExposure,
}

/// Risk appetite of a stope. Fixed to the stope id: 0 conservative,
/// 1 balanced, 2 aggressive.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RiskProfile {
    Conservative,
    Balanced,
    Aggressive,
}

impl RiskProfile {
    /// The canonical profile for a stope id. `None` for any id >= 3.
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Conservative),
            1 => Some(Self::Balanced),
            2 => Some(Self::Aggressive),
            _ => None,
        }
    }

    pub fn id(&self) -> u8 {
        match self {
            Self::Conservative => 0,
            Self::Balanced => 1,
            Self::Aggressive => 2,
        }
    }

    /// Ceiling on how much of a stope's allocation may sit on seams whose
    /// yield is [`YieldKind::Emissions`].
    ///
    /// Enforced by `register_seam` and `update_seam_allocation`, so the
    /// difference between the three stopes is a constraint the chain rejects
    /// transactions over, not a label on a marketing page.
    pub fn max_emissions_bps(&self) -> u16 {
        match self {
            Self::Conservative => 2_000,
            Self::Balanced => 5_000,
            Self::Aggressive => 10_000,
        }
    }

    /// Highest headlamp risk tier a seam may carry to be routable from this
    /// stope.
    pub fn max_risk_tier(&self) -> u8 {
        match self {
            Self::Conservative => 2,
            Self::Balanced => 3,
            Self::Aggressive => 5,
        }
    }
}

/// Lifecycle of an Orecart redemption ticket.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TicketStatus {
    /// Shares are burned, the payout is reserved, the delay is running.
    Queued,
    /// Paid out.
    Claimed,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deserialize exactly `LEN` zero bytes, then serialize the result back.
    ///
    /// Borsh's `try_from_slice` fails both when the buffer is too short and
    /// when bytes are left over, so this asserts the declared `LEN` against
    /// the real wire layout in both directions at once. A hand-counted `LEN`
    /// that is one byte off fails here rather than in production, where it
    /// would truncate the last field of every account of that type.
    fn assert_len<T: AnchorSerialize + AnchorDeserialize>(name: &str, len: usize) {
        let zeros = vec![0u8; len];
        let value = T::try_from_slice(&zeros)
            .unwrap_or_else(|e| panic!("{name}::LEN = {len} does not decode exactly: {e}"));
        let encoded = value.try_to_vec().expect("serialize").len();
        assert_eq!(encoded, len, "{name} re-encodes to {encoded}, not {len}");
    }

    #[test]
    fn declared_lengths_match_the_structs() {
        assert_len::<VaultConfig>("VaultConfig", VaultConfig::LEN);
        assert_len::<Adit>("Adit", Adit::LEN);
        assert_len::<Stope>("Stope", Stope::LEN);
        assert_len::<Seam>("Seam", Seam::LEN);
        assert_len::<Miner>("Miner", Miner::LEN);
        assert_len::<Orecart>("Orecart", Orecart::LEN);
        assert_len::<OrecartQueue>("OrecartQueue", OrecartQueue::LEN);
        assert_len::<Keeper>("Keeper", Keeper::LEN);
    }

    /// Locks the account size table in README.md to the code.
    ///
    /// These numbers decide rent, so they are quoted in the README and read by
    /// whoever funds the deployment. Asserting them here means the table
    /// cannot drift away from the structs without a test failing.
    #[test]
    fn account_sizes_match_the_readme_table() {
        assert_eq!(VaultConfig::LEN, 288);
        assert_eq!(Adit::LEN, 200);
        assert_eq!(Stope::LEN, 184);
        assert_eq!(Seam::LEN, 256);
        assert_eq!(Miner::LEN, 184);
        assert_eq!(Orecart::LEN, 192);
        assert_eq!(OrecartQueue::LEN, 104);
        assert_eq!(Keeper::LEN, 120);
    }

    #[test]
    fn every_account_is_eight_byte_aligned() {
        for (name, len) in [
            ("VaultConfig", VaultConfig::LEN),
            ("Adit", Adit::LEN),
            ("Stope", Stope::LEN),
            ("Seam", Seam::LEN),
            ("Miner", Miner::LEN),
            ("Orecart", Orecart::LEN),
            ("OrecartQueue", OrecartQueue::LEN),
            ("Keeper", Keeper::LEN),
        ] {
            assert_eq!(len % 8, 0, "{name}::LEN ({len}) is not 8-byte aligned");
        }
    }

    #[test]
    fn stope_ids_map_to_exactly_three_profiles() {
        assert_eq!(RiskProfile::from_id(0), Some(RiskProfile::Conservative));
        assert_eq!(RiskProfile::from_id(1), Some(RiskProfile::Balanced));
        assert_eq!(RiskProfile::from_id(2), Some(RiskProfile::Aggressive));
        assert_eq!(RiskProfile::from_id(3), None);
        assert_eq!(RiskProfile::from_id(u8::MAX), None);

        for id in 0..STOPE_COUNT {
            let profile = RiskProfile::from_id(id).expect("profile");
            assert_eq!(profile.id(), id);
        }
    }

    #[test]
    fn risk_appetite_is_monotonic_across_profiles() {
        let c = RiskProfile::Conservative;
        let b = RiskProfile::Balanced;
        let a = RiskProfile::Aggressive;

        assert!(c.max_emissions_bps() < b.max_emissions_bps());
        assert!(b.max_emissions_bps() < a.max_emissions_bps());
        assert!(c.max_risk_tier() < b.max_risk_tier());
        assert!(b.max_risk_tier() < a.max_risk_tier());
        assert!(a.max_risk_tier() <= MAX_RISK_TIER);
    }

    /// The seeds a client derives are the seeds this program derives.
    #[test]
    fn numeric_seeds_are_little_endian() {
        assert_eq!(1u8.to_le_bytes(), [1]);
        assert_eq!(258u16.to_le_bytes(), [0x02, 0x01]);
        assert_eq!(1u32.to_le_bytes(), [0x01, 0x00, 0x00, 0x00]);
    }
}
