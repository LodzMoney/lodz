# LODZ Risk Specification

Version 1. Last reviewed 2026-08-15.

This document specifies how LODZ measures, tiers and discloses risk. It is normative: the
figures and rules here are the ones the program enforces and the API serves. Where a claim
is measured, the measurement and its source are named. Where something is unverified, it
says so and stays unverified.

Companion documents: `seam-spec.md` (yield classification and display rules),
`architecture.md` (system structure), `security.md` (implementation hardening).
All measurements trace to `docs/research/btc-on-solana.md`.

---

## 1. Scope and the claim this protocol does not make

LODZ routes tokenized representations of bitcoin on Solana. Every asset it accepts is a
claim issued by a third party against bitcoin held somewhere else. None of them is a coin
on the Bitcoin base chain, and the protocol never describes them as one.

This is enforced in the type system rather than left to copy. The on-chain `CustodyKind`
enum (`packages/anchor-program/programs/lodz-vault/src/state/mod.rs:138-149`) carries a
doc comment stating that no variant is a coin on the Bitcoin network, and its three
variants each name what the depositor is exposed to instead:

| Variant | Exposure |
|---|---|
| `BridgeMinted` | The bridge's validator set and its contracts |
| `CustodianRedeemable` | A named custodian and its published redemption process |
| `SyntheticExposure` | Whatever mechanism maintains the peg |

Principal is at risk. Deposits carry no insurance and can be impaired by a venue exploit,
an issuer failure or a custody failure. No LODZ interface, document or marketing asset
describes any position as free of risk or as carrying an assured return, at any figure.
That prohibition is absolute rather than conditional on how good the numbers look.

---

## 2. Two tier scales, and why they are not the same scale

LODZ uses two distinct risk scales. Conflating them is a spec error.

### 2.1 On-chain tier: `u8`, range 1 to 5

Declared at `state/mod.rs:103-104`:

```rust
pub const MIN_RISK_TIER: u8 = 1;
pub const MAX_RISK_TIER: u8 = 5;
```

The accompanying comment states the reason there is no tier 0: every representation of
bitcoin on Solana carries bridge or custody risk, and a zero would read as "none". The
scale therefore starts at 1 by construction, not by convention.

Both `Adit` (one accepted asset, `state/adit.rs:62-63`) and `Seam` (one venue-asset-kind
triple, `state/seam.rs:29-31`) carry a `risk_tier: u8` in this range.

The bound is enforced on registration, not assumed. `register_adit`
(`instructions/admin.rs:260-261`) and `register_seam` (`instructions/admin.rs:476-477`)
both check:

```rust
require!(
    params.risk_tier >= MIN_RISK_TIER && params.risk_tier <= MAX_RISK_TIER,
    LodzError::InvalidRiskTier
);
```

`LodzError::InvalidRiskTier` (`errors.rs:55-56`) carries the message
`"Risk tier is outside the 1..=5 headlamp range."`

### 2.2 Off-chain tier: `low` / `medium` / `high`

Declared at `apps/service/src/models/common.py:30` as
`RiskTier = Literal["low", "medium", "high"]`, ordered by
`RISK_TIER_ORDER = {"low": 0, "medium": 1, "high": 2}` (`common.py:49`).

This is the scale used by the disclosure layers served at `GET /headlamp/risk`. It is
coarser on purpose: a layer such as "custody of the underlying" is not a per-position
number and a five-point scale would imply a precision the evidence does not support.

### 2.3 The binding rule between them

A seam's numeric tier is not free. Each stope declares a `RiskProfile`, and the profile
caps the tier of any seam routable from it (`state/mod.rs:195-201`):

| Stope | `RiskProfile` | `max_risk_tier()` | `max_emissions_bps()` |
|---|---|---|---|
| 0 | `Conservative` | 2 | 2000 |
| 1 | `Balanced` | 3 | 5000 |
| 2 | `Aggressive` | 5 | 10000 |

Enforced at `instructions/admin.rs:495-496`:

```rust
require!(
    params.risk_tier <= stope.risk_profile.max_risk_tier(),
    LodzError::RiskTierExceedsStopeProfile
);
```

The difference between the three stopes is therefore a constraint the chain rejects
transactions over, not a label on a page. The source comment at `state/mod.rs:181-184`
states exactly this.

---

## 3. Risk layers

The service implements seven layers, served by `risk_layers()` in
`apps/service/src/services/headlamp_risk.py` and shaped by `RiskLayer` in
`apps/service/src/models/headlamp.py`. They are ordered by distance from the depositor.

The five categories most often named in BTC yield disclosures -- bridge, custody,
protocol, oracle, liquidity -- map onto this set as follows: bridge risk lives in
`issuance` (wrap hops, mint authority), custody risk is `custody`, protocol risk is
`protocol`, oracle risk is a factor inside `protocol` and again inside `operational`, and
liquidity risk is inside `market`. Two further layers exist because the research produced
findings that do not fit any of the five: `emissions` and `data-integrity`.

Every layer carries a tier, a summary, and a list of `RiskFactor` entries. A factor
carries its own tier, a detail string stating the measurement, and where applicable a
`source_url`. The overall tier returned by `/headlamp/risk` is the worse of the layer
maximum and the tier of the allocation being asked about.

### 3.1 `issuance` -- the claim itself

Tier: medium.

| Factor | Tier | Observable indicator |
|---|---|---|
| `mint-authority-is-a-key` | high | Mint authority classified on-curve by RPC read. True for cbBTC and xBTC |
| `freeze-authority` | high | Freeze authority present and on-curve. True for cbBTC and xBTC |
| `wrap-hops` | medium | `wrap_hops` field. 2 for WBTC (Portal), 1 for the rest |
| `ticker-collision` | medium | Two 8-decimal Solana tokens named WBTC, one denylisted |

The on-curve / off-curve test is the discriminator. A key on the ed25519 curve can exist
as a private key, so an institution holds it. An address off the curve is a program
derived address, so only a program can sign. Source: `https://api.mainnet-beta.solana.com`
via `getAccountInfo(jsonParsed)`, verified 2026-08-15T07:00Z.

The honest limit of this test is recorded rather than hidden: an on-curve authority may
still be held under multisig, HSM or MPC. The test separates "enforced by an on-chain
program" from "dependent on off-chain operational procedure". It does not grade key
management quality.

### 3.2 `custody` -- the bitcoin behind the claim

Tier: medium. LODZ never holds the underlying and cannot recover it for a holder.

| Factor | Tier | Observable indicator |
|---|---|---|
| `por-coverage` | high | `por_type` per asset. The two largest, cbBTC and WBTC, are type E |
| `issuer-survival` | medium | soBTC trades 99.96 percent below bitcoin with 16,149 units still on chain |
| `cross-chain-supply` | medium | zBTC also exists on other chains through CCIP |

Proof-of-reserve maturity is graded A through E (`common.py:40`):

| Type | Meaning |
|---|---|
| A | On-chain automated attestation, a feed anyone can read |
| B | Periodic audit |
| C | Merkle proof of reserves |
| D | Protocol-intrinsic, collateral verifiable from protocol state |
| E | No verifiable proof, issuer disclosure only |

The uncomfortable result is stated plainly: the two assets with automated feeds, zBTC and
xBTC, are the two smallest. The two largest have no automated proof on any chain.

`cross-chain-supply` matters more than it looks. The zBTC reserve feed reports total
reserves while Solana supply is only part of the total, so comparing one chain's supply
against a global feed does not settle whether backing is one to one. This is recorded as
a limit on the check, not as a finding against the asset.

### 3.3 `protocol` -- venue contracts

Tier: medium. Each seam is a position inside another party's program.

| Factor | Tier | Observable indicator |
|---|---|---|
| `clmm-range` | medium | Every liquidity seam is a concentrated position that stops earning out of range |
| `strategy-layer` | medium | The Kamino liquidity seam adds a manager contract above the pool |
| `oracle-dependency` | high | Lending venues liquidate against price feeds |

`oracle-dependency` is tiered high on evidence, not caution. Mango Markets V3 lost $115M
to oracle manipulation on 2022-10-11, and the Drift loss of $295M on 2026-04-01 combined a
compromised admin with a faked token price. This is a live technique.

### 3.4 `market` -- liquidity and divergence

Tier: high. This is the highest-tier layer in the set.

| Factor | Tier | Observable indicator |
|---|---|---|
| `divergence-loss` | high | DefiLlama publishes no divergence figure for any BTC pool on Solana |
| `spot-rate-artifacts` | medium | The cbBTC/USDC pool printed 74,187 percent on one day of 646 |
| `exit-depth` | high | About $24M of BTC LP liquidity on Solana across 62 pools |
| `lending-pays-nothing` | medium | $75.4M in lending; Kamino cbBTC 3.2 percent utilised, pays 0.00459 percent |

Every published LP rate in this space is gross of divergence loss. LODZ estimates the loss
from pool price history where it can and reports the gap as unknown where it cannot. It
does not fabricate an estimate to fill the field.

`exit-depth` is the mechanism that lengthens the redemption queue under stress: a
redemption large relative to a pool moves price against itself.

Source for the lending figure:
`https://api.kamino.finance/kamino-market/7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF/reserves/metrics`

### 3.5 `emissions` -- incentive programmes

Tier: low, because the measured exposure is zero.

| Factor | Tier | Observable indicator |
|---|---|---|
| `zero-emissions-verified` | low | 94 BTC pools, zero paying a reward token; 647 days of history on the two largest |
| `points-not-priced` | medium | Points programmes are not converted to a rate |

The zero is a measurement, not missing data. The same DefiLlama snapshot shows 15 non-BTC
Solana pools paying rewards, so reward collection is working. Source:
`https://yields.llama.fi/pools`.

`points-not-priced` is a deliberate omission with a stated cost: where a venue runs a
points programme, that yield is absent from LODZ figures rather than estimated into them.
Assigning a price to unissued points would move an emissions expectation into a
sustainable number. The consequence is that reported yield can understate what a
depositor eventually receives, and understating is the direction this protocol errs in.

### 3.6 `operational` -- LODZ itself

Tier: medium. The router, the keeper and the redemption queue are LODZ code.

| Factor | Tier | Observable indicator |
|---|---|---|
| `program-status` | medium | Reads `settings.vault_is_live`. Currently reports the program is not deployed |
| `upgrade-and-oracle` | high | LODZ will hold an upgrade authority and will read external prices |
| `keeper-liveness` | medium | Rebalancing and unwinding depend on a keeper running |

`upgrade-and-oracle` names LODZ's own exposure to the exact technique that took $295M from
Drift: an upgrade authority plus an external price source are the same two surfaces. This
is disclosed against the protocol rather than only against its venues.

`program-status` is computed, not written. While the vault is not deployed the service
adds a disclosure stating that every projection models what the current catalogue would
pay and none of it reports realised performance.

### 3.7 `data-integrity` -- signals that look healthy and are not

Tier: medium. This layer exists because the research caught three separate cases.

| Factor | Tier | Observable indicator |
|---|---|---|
| `200-is-not-alive` | medium | `api.zeusscan.io` answers 200 on every path with a parking page |
| `tvl-is-not-alive` | high | Drift BTC-PERP: 250 BTC open interest, no funding update since April |
| `source-divergence` | medium | Rates cross-checked against each venue's own API |

`tvl-is-not-alive` carries two independent examples: the Drift market above, and Kamino's
fBTC reserve holding $5.48M with zero borrows and paying zero. Size proves neither
activity nor yield.

`source-divergence` is parameterised by `settings.source_divergence_bps`. A relative gap
above that threshold suppresses the seam entirely, on the reasoning that one of the two
sources is stale and there is no way to tell which.

---

## 4. Asset table

The routable set is defined in `apps/service/src/config/btc_assets.py`. Every field was
read from Solana mainnet at `VERIFIED_AT = 2026-08-15T07:00Z` by the method recorded in
`VERIFICATION_METHOD`: RPC `getTokenSupply` and `getAccountInfo(jsonParsed)`, on-curve /
off-curve classification of authorities, and a Wormhole `wrapped_meta` decode for the
Portal asset.

| Asset | Mint | Issuer | Trust model | Hops | Mint auth is key | Freezable | PoR | Supply |
|---|---|---|---|---|---|---|---|---|
| cbBTC | `cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij` | Coinbase | custodial | 1 | true | true | E | 3396.35594396 |
| WBTC | `3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh` | BitGo via Wormhole Portal | bridged | 2 | false | false | E | 2631.90720452 |
| zBTC | `zBTCug3er3tLyffELcvDNrKkCymbPWysGcWihESYfLg` | Zeus Network | program-controlled | 1 | false | false | A | 59.67314796 |
| xBTC | `CtzPWv73Sn1dMGVU3ZtLv9yWSyUAanBni19YWDaznnkn` | OKX | custodial | 1 | true | true | A | 352.07746268 |

Asset tiers assigned in `headlamp_risk._ASSET_TIER`: cbBTC low, WBTC medium, xBTC medium,
zBTC high. The ordering is derived from measured properties rather than reputation. zBTC
has the best on-chain properties in the table and the highest tier, because a $3.75M float
with $96K of DEX liquidity and zero audits listed on DefiLlama dominates the structural
advantage.

Reserve feeds, where they exist:

- zBTC: `arbitrum:0xd9344493d99153Ad4353D604A1d80d4089004c5D`, a Chainlink proof-of-reserve
  proxy read directly and reporting 59.72 BTC.
- xBTC: `chainlink-datastreams:0x0009b402d77df149a0d5ce37220cc175f17e2cf59a9952b27abf2f335acac999`.
  OKX's exchange-level proof of reserve does not cover this token.

The three assets chosen span three distinct trust models -- custodial, bridged and
program-controlled. That spread is what makes the disclosure meaningful: a catalogue of
four custodial tokens could not demonstrate a difference in trust model to a depositor.

### 4.1 The WBTC collateral path, proved rather than asserted

The `wrap_hops = 2` value for WBTC is not an editorial judgement. It was established
on-chain:

1. The mint authority of `3NZ9JMV...` is `BCD75RNBHrJJpW4dXVagL5mPjzRLnVZq4YirJdjEYMV7`.
2. `find_program_address([b"mint_signer"], wormDTUJ6AWPNvk59vGQbDvGJmqbDTdgWgAqcLBCgUb)`
   re-derives that same address at bump 254. It is the Wormhole Token Bridge PDA.
3. Decoding the `wrapped_meta` account gives origin chain 2 (Ethereum) and origin token
   `0x2260fac5e5542a773aa44fbcfedf7c193bc2c599`, which is Ethereum WBTC.

The collateral path is therefore bitcoin to BitGo custody, to Ethereum WBTC, to Solana
through Portal. Each hop is an independent failure point, and Portal is the bridge that
lost $326M on 2022-02-02.

---

## 5. Exclusions

### 5.1 Denylist -- never accepted

Keyed by mint in `BTC_DENYLIST`, because the symbols collide with allowed assets.

| Mint | Label | Reason |
|---|---|---|
| `9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E` | soBTC (Sollet) | Depegged 99.96 percent, issuer defunct |
| `21BTCo9hWHjGYYUQQLqjLgDBxjcn8vDt4Zic7TB3UbNE` | 21BTC | Economically extinct |
| `5XZw2LKTyrfvfiskJ78AMpackRjPcyCif1WhUsPDuVqQ` | WBTC (BitGo canonical) | $46K Solana liquidity, unusable exit depth |

Three properties of soBTC make it the reason the denylist is keyed by mint:

- Its on-chain symbol is plain `BTC`. Anything that accepts SPL tokens by symbol accepts it.
- It has 6 decimals where every allowed asset has 8. Shared arithmetic misplaces amounts
  by a factor of 100.
- Four markets still exist for it on Save. A loop that enumerates a venue's markets to
  build seams pulls all four in.

The third denylist entry is the sharpest case: BitGo's canonical Solana WBTC is a sound
asset with a single custody hop, excluded purely because $46K of liquidity means any exit
of meaningful size moves price against itself. Structural quality does not override exit
depth.

### 5.2 Known but not routed

`NOT_ROUTED` records representations that exist and are deliberately excluded, so the risk
page can explain the absence rather than leave a reader to infer it.

| Mint | Asset | Reason |
|---|---|---|
| `6DNSN2BJsaPFdFFc1zP37kkeNe4Usc1Sqkzr9C9vPWcU` | tBTC | Best structural proof of reserve of any candidate, but 21.6 BTC on Solana and $25K DEX liquidity. Arrives through a Wormhole gateway, so the protocol-intrinsic guarantee does not survive the trip |
| `LBTCgU4b3wsFKsPwBn1rRZDx5DoFutM6RPiEt1TPDsY` | LBTC | Already yield bearing, which would make decomposition two levels deep. Every Solana venue listing it pays zero supply APY |
| `3orqhCKM5admbcHkHQhRAEKbXhUT5VPgsQqz7fBa6QdF` | FBTC | 39 holders on Solana; $5.48M in a Kamino reserve with zero borrows, which is deposits without a market |

### 5.3 Resolution is fail-closed

`assert_routable(mint)` raises rather than returning `None`. The source comment states the
reason: every call site that routes capital has to stop, and a `None` that gets ignored is
how a denylisted mint ends up in a position. An unknown mint produces an error naming the
file that must be edited, with on-chain measurements, before routing can use it.

---

## 6. Incident record

Served by `headlamp_risk.incidents()`, filtered from the DefiLlama hacks dataset to
entries that map onto this product. Two of the six are not history.

| Incident | Date | Loss | Technique | Returned | Maps to |
|---|---|---|---|---|---|
| Portal (Wormhole) | 2022-02-02 | $326M | Signature verification bypass | Yes | The bridge issuing WBTC in this catalogue |
| Drift Trade | 2026-04-01 | $295M | Compromised admin plus fake token price | No | The excluded basis venue, and LODZ's own surfaces |
| FTX | 2022-11-12 | $450M | Private key compromise | No | The Sollet operator, hence soBTC on the denylist |
| Mango Markets V3 | 2022-10-11 | $115M | Price oracle manipulation | No | Collateral valuation, hence dual-source rates |
| Ronin Bridge | 2022-03-23 | $624M | Validator key compromise via social engineering | No | Permissioned validator sets, structurally zBTC's MPC |
| Multichain | 2023-07-07 | $126M | Private key compromise | No | Single operator dependence, as with cbBTC or xBTC mint keys |

Two entries carry qualifications that are part of the record:

- The Portal loss was covered in full by a backer. That is a fact about the backer, not a
  property of the bridge, and the disclosure says so.
- FTX is on this list although nothing was exploited on chain. The issuer stopped existing.
  Issuer survival is therefore its own risk factor under `custody` rather than a footnote
  to exploits.

---

## 7. On-chain enforcement

The following are rejected by the program, not discouraged by policy.

### 7.1 An unregistered mint cannot be deposited

There is one `Adit` per mint, and a mint without an adit cannot be deposited
(`state/adit.rs:7-9`). Registration requires `custody_kind` and `risk_tier` to be written
on chain first. The gate is therefore: no token enters without its custody model and risk
tier recorded.

`Adit` also pins `token_program` at registration (`state/adit.rs:20-28`) and re-checks it
on every deposit and payout transfer, which is what makes routing a Token-2022 mint
through the classic token program impossible.

### 7.2 Non-1:1 representations are expressed, not assumed

`conversion_num` and `conversion_den` (`state/adit.rs:33-43`) fold together the decimal
difference against the internal 8-decimal unit and the asset's declared ratio to one
bitcoin. A representation that is not 1:1 is expressed here rather than quietly counted as
if it were. `INTERNAL_DECIMALS = 8` carries an explicit comment (`state/mod.rs:90-95`)
stating that nothing about the unit makes a deposit bitcoin.

### 7.3 Authority ceilings a compromised key cannot raise

Declared at `state/mod.rs:64-85` under a comment stating these are the parameters where
the difference between bad and unrecoverable is decided:

| Constant | Value | Effect |
|---|---|---|
| `MAX_FEE_BPS` | 500 | Redemption fee cannot exceed 5 percent |
| `MAX_BASE_REDEMPTION_DELAY_SEC` | 30 days | Above this it is a lockup, which this product does not sell |
| `MAX_TOTAL_REDEMPTION_DELAY_SEC` | 180 days | Ceiling including the queue congestion term |
| `MAX_KEEPER_UNBOND_COOLDOWN_SEC` | 30 days | Keeper unbond cooldown ceiling |

The authority sets live values below these. It cannot raise the ceilings by sending a
transaction. The stated threat model is explicit: a compromised authority key is a
realistic failure mode for a young protocol.

### 7.4 Emissions disclosure is a validity condition

`Seam::validate_emission_fields` (`state/seam.rs:85-108`) rejects an emissions seam that
does not declare when its emission ends, or declares an end already in the past, or fails
to name the mint the emission is paid in. It also rejects a sustainable seam carrying
emission fields, so the two kinds cannot be blurred by stale values.

`Seam::accrual_window_open` (`state/seam.rs:111-116`) closes the window once the chain
clock passes `emission_ends_at`, so a seam cannot keep booking yield from a schedule that
has run out. The corresponding error is `EmissionEnded` (`errors.rs:81-82`).

`yield_kind` is immutable after registration. The source states the reason
(`state/seam.rs:26-28`): changing it would silently rewrite the meaning of
`realized_yield` already booked under the old kind.

### 7.5 The two-kind on-chain enum against the three-kind catalogue

The on-chain `YieldKind` has two variants, `Sustainable` and `Emissions`
(`state/mod.rs:117-130`). The service-side `YieldKind` has three, adding `counterparty`
(`common.py:25`).

This divergence is intentional and is recorded here so it is not read as a defect. The
third kind exists because the highest advertised BTC yield on Solana is a perpetuals vault
paying out of trader losses. Such a venue is classified and displayed by the catalogue but
is not routable by the deployed program, which can only register a seam of one of its two
variants. Any future support for counterparty seams on chain requires an enum change and
therefore a program upgrade, which is the correct level of friction for adding a yield
source whose payer is another trader.

---

## 8. Exposure metrics

`RiskSummary` (`models/headlamp.py:86-103`) is embedded in every assay response so a rate
cannot be rendered without its risk context. Fields:

| Field | Meaning |
|---|---|
| `overall_tier` | Tier of the allocation asked about |
| `exposure_by_tier_bps` | Share of routed capital per risk tier, in basis points |
| `exposure_by_trust_model_bps` | Share behind each custody arrangement |
| `max_wrap_hops` | Deepest custody chain any routed capital sits behind |
| `freezable_exposure_bps` | Share in assets whose issuer can freeze accounts |
| `layers` | Layer headlines, factors stripped |
| `disclosures` | The standing disclosure list |

`overall_tier` in the summary describes the allocation, not the maximum across layers. The
source comment (`headlamp_risk.summary_for`) gives the reason: folding in risks that exist
at any size would pin every answer to high and make the field say nothing. The full
breakdown with factors is served at `GET /headlamp/risk`.

`freezable_exposure_bps` deserves emphasis. It answers a question a depositor cannot
otherwise ask: what share of my capital sits in tokens whose issuer can immobilise the
vault's account without any technical failure occurring. For a catalogue containing cbBTC
and xBTC this is a non-zero number by construction.

---

## 9. Standing disclosures

Returned by `headlamp_risk.disclosures()` with every risk and assay response:

1. Every asset routed is a wrapped or bridged claim on bitcoin held by a third party. It is
   not a coin on the Bitcoin base chain, and its value depends on that third party
   honouring redemption.
2. Yield is reported in three kinds and they are never added into one another.
3. Emissions exposure across this catalogue is zero, measured rather than assumed.
4. Liquidity provision fee rates are quoted gross of divergence loss by every upstream
   source. Where LODZ can estimate that loss it subtracts it; where it cannot, the seam is
   marked and no net figure is given.
5. BTC lending on Solana pays close to nothing. The largest reserve, $44.0M of cbBTC on
   Kamino, pays 0.00459 percent. Any product advertising a meaningful BTC lending rate on
   this chain is describing something else.
6. Principal is at risk. Deposits are not bank deposits, carry no insurance, and can be
   impaired by a venue exploit, an issuer failure or a custody failure.
7. Redemption is a claim on the vault's open positions. Its speed depends on venue depth at
   the time of exit, and the queue lengthens under stress.

While `vault_is_live` is false, an eighth is appended stating that the vault program is not
deployed and every projection models what the current catalogue would pay rather than
reporting realised performance.

---

## 10. Unverified

Recorded as unverified rather than filled in. Each entry names what would settle it.

| Item | Status | What would settle it |
|---|---|---|
| zBTC guardian m-of-n threshold | Unverified | Not published. Requires Zeus disclosure or program account decoding |
| zBTC supply summed across all chains | Unverified | Token also exists on other chains through CCIP; a 1:1 comparison needs the total |
| Drift `Custom: 101` error meaning | Unverified | Drift error code table. Determines whether the market is halted or the oracle is stale |
| Zeta, Adrena, Flash Trade BTC perp liveness | Unverified | Apply the same on-chain check used on Drift |
| Points programme sizes | Unverified | No unauthenticated public API found. Until then, "zero emissions" carries the qualifier "token emissions" |
| xBTC designated locked BTC address | Unverified | OKX disclosure |
| cbBTC audit cadence and format | Unverified | Coinbase documentation |
| 21BTC custodian identity | Unverified | 21.co states "institutional-grade third-party" only |

The upgrade authority survey in `docs/research/btc-on-solana.md` section 1-4 produced one
finding worth restating: every relevant bridge and issuer program examined is upgradeable,
and every upgrade authority is an off-curve PDA. No program in the set has burned its
authority. A PDA mint authority is therefore a conditional guarantee, conditional on
whoever controls the program that drives it, and the risk display states the upgrade
surface alongside the mint authority for that reason.
