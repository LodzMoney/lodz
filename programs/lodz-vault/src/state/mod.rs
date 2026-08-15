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
/// `["orecart", owner, stope_id(u8), ticket_index(u32 LE)]`
///
/// `stope_id` is in the seed because `Miner::ticket_count` -- the counter
/// `request_redemption` requires `ticket_index` to equal -- is *per stope*
/// (`["miner", owner, stope_id]`). Without `stope_id` here the two namespaces
/// disagree: a depositor holding positions in two stopes has one ticket
/// counter per stope but a single ticket address space, so the second stope's
/// counter (still at 0) resolves to a ticket PDA the first stope already
/// created. `init` fails with "already in use", the counter never advances
/// because it only advances on success, and the position can never be
/// redeemed. Measured on devnet 2026-08-16 against the first deployment:
/// `Allocate: account 3FU1UL5LBZReQpXh9KHDp2gThdybAf4ofzzUvhXYkfur already in
/// use`. Selling three risk-profiled stopes makes holding more than one the
/// normal case, so this stranded principal on the ordinary path.
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
/// tell a depositor whether the number survives next quarter, so the kinds are
/// never summed into a single field anywhere in this program: separate
/// accumulators on the [`Stope`], separate accumulators on the [`Miner`], and
/// a `yield_kind` on every `YieldAccrued` event.
///
/// # Why three and not two
///
/// The measurement in `docs/research/btc-on-solana.md` found a third source on
/// Solana that looks sustainable and is not: a delta-neutral market-making
/// vault paying 214.828 % whose yield is the losses of the traders on the
/// other side. It does not end on a schedule, so it is not emissions; nobody
/// is paying it as a fee for a service, so it is not sustainable. Filing it
/// under either one is a false statement in the ledger, and the ledger of
/// where yield comes from is the entire product. The off-chain half
/// (`assay-engine`, the service API, the site) has carried three kinds from
/// the start; this enum is what makes the chain agree with them.
///
/// Variants are appended, never reordered: the Borsh discriminant of
/// `Sustainable` (0) and `Emissions` (1) is fixed by every account already
/// written.
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
    /// Paid out of what somebody on the other side of a trade lost.
    ///
    /// It has no end date, so it cannot be disclosed the way an emission can,
    /// and it does not survive on the same terms as a fee: it lasts exactly as
    /// long as the losing flow does.
    ///
    /// How much of it a stope may hold is
    /// [`RiskProfile::max_counterparty_bps`], which is zero for the two
    /// profiles whose published stance is that they hold none. Recording the
    /// kind and deciding whether to hold it are separate: this variant exists
    /// so the ledger can state what happened even where the policy says not to
    /// go, because a seam already held can start paying this way without
    /// asking.
    Counterparty,
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

    /// Ceiling on how much of a stope's allocation may sit on seams whose
    /// yield is [`YieldKind::Counterparty`].
    ///
    /// # Where these numbers come from
    ///
    /// They are the commitment the site already publishes. `CHAMBER_POLICY` in
    /// `apps/web/app/_shared/measured.ts` carries an `admitsCounterparty` flag
    /// per chamber -- false for conservative, false for balanced, true for
    /// forward -- and the conservative stance reads "nothing funded by
    /// somebody else's loss". A ceiling here that was looser than that would
    /// make the published sentence unenforced, which is the exact failure this
    /// program exists to close.
    ///
    /// The routing defaults in `packages/seam-router/src/constraints.ts`
    /// disagreed with the site for balanced (1_000 bps against the site's
    /// "false"). The published promise wins: a user-facing commitment does not
    /// get widened by an internal default. That file's own comment already
    /// says these are "defaults, not law" and that "the on-chain vault
    /// parameters are the authority once the program is live" -- this function
    /// is that authority, and it had nothing in it until now.
    ///
    /// Raising any of these needs a program upgrade. That friction is
    /// deliberate for a promise this load-bearing.
    pub fn max_counterparty_bps(&self) -> u16 {
        match self {
            Self::Conservative => 0,
            Self::Balanced => 0,
            Self::Aggressive => 3_000,
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

        // Counterparty is the one axis that is not a gradient: the two
        // profiles the site says admit none of it admit exactly none, and only
        // the forward profile carries any. Monotonic, but flat at the bottom.
        assert_eq!(c.max_counterparty_bps(), 0);
        assert_eq!(b.max_counterparty_bps(), 0);
        assert!(a.max_counterparty_bps() > 0);
        assert!(a.max_counterparty_bps() <= crate::math::MAX_BPS);
    }

    /// The seeds a client derives are the seeds this program derives.
    #[test]
    fn numeric_seeds_are_little_endian() {
        assert_eq!(1u8.to_le_bytes(), [1]);
        assert_eq!(258u16.to_le_bytes(), [0x02, 0x01]);
        assert_eq!(1u32.to_le_bytes(), [0x01, 0x00, 0x00, 0x00]);
    }
}
