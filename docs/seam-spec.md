# LODZ Seam Specification

Version 1. Last reviewed 2026-08-15.

A seam is one yield venue LODZ can route BTC collateral into. This document specifies the
seam schema, how a yield source is classified, how measurements are collected and
cross-checked, and which display rules the service enforces before any rate reaches a
caller.

It is normative and it is written against the running implementation. The schema section
is a field-by-field transcription of `apps/service/src/models/seam.py`; the rule
thresholds are the live values served by `GET /health/detailed`; the catalogue table is a
capture of `GET /seams` at the timestamp given. Where the specification describes a rule
that is not yet enforced in code, it says so in that place rather than implying coverage.

Companion documents: `risk-spec.md` (risk tiering and disclosure), `architecture.md`
(system structure), `security.md` (hardening). Measurements trace to
`docs/research/btc-on-solana.md` sections 2, 3 and 7.

---

## 1. Units and conventions

| Convention | Rule |
|---|---|
| Basis points | 1 bps = 0.01 percent. `BPS = 10_000` (`models/common.py:53`) |
| Rate fields | Every rate is carried twice: `*_bps` as an integer and `*_pct` as an exact float |
| Precision | The percentage is the precise field. The integer is a convenience and may round to zero |
| USD | `tvl_usd` is a float in whole US dollars |
| Timestamps | ISO 8601, UTC |
| Asset identity | `asset_mint`. The `asset` string is display only |

The dual representation is not redundancy. Several real rates in this catalogue sit far
below one basis point: Kamino cbBTC pays 0.00459 percent, which is 0.46 bps and rounds to
zero as an integer. That measurement is the entire argument that BTC lending on Solana is
not a product, so erasing it through rounding would delete the most important number in
the set. `rounds_to_zero_bps` exists to tell a caller that the display rate is non-zero
but its integer form is zero, and that it must render the percentage rather than `0`.

---

## 2. Seam schema

Transcribed from `apps/service/src/models/seam.py`. Types are as declared; `Optional`
means the field is nullable and defaults to null unless stated.

### 2.1 Identity

| Field | Type | Notes |
|---|---|---|
| `id` | `str` | Stable slug. Join key for the SDK and CLI |
| `name` | `str` | Human readable name of the position |
| `venue` | `str` | Protocol the position is held on |
| `asset` | `str` | Symbol of the BTC representation. **Display only** |
| `asset_mint` | `Optional[str]` | Mint of the BTC leg. The only correct identifier. Null when the BTC leg is an aggregate of several markets |
| `kind` | `SeamKind` | `lending`, `lp` or `perp_vault` |
| `yield_type` | `YieldKind` | `sustainable`, `emissions` or `counterparty` |

`asset` is explicitly not an identifier. Solana carries two 8-decimal tokens named WBTC
with different collateral paths, one of which is denylisted, so any code resolving an
asset from the string picks wrong roughly half the time.

### 2.2 Rates

| Field | Type | Notes |
|---|---|---|
| `apy_bps` | `int`, >= 0 | Spot rate as reported upstream. Present for completeness. **Not for rendering** |
| `apy_pct` | `float`, >= 0 | The same spot rate, exact |
| `rounds_to_zero_bps` | `bool` | True when the display rate is non-zero but rounds to 0 bps |
| `apy_7d_bps` | `Optional[int]` | Seven day average |
| `apy_30d_mean_bps` | `Optional[int]` | Thirty day mean |
| `apy_90d_median_bps` | `Optional[int]` | Ninety day median. A median so one artifact day cannot move it |
| `display_apy_bps` | `int`, >= 0 | **The only rate a caller should render** |
| `display_apy_pct` | `float`, >= 0 | The same, exact |
| `display_apy_basis` | `DisplayApyBasis` | Which window produced it |

`DisplayApyBasis = Literal["apy_7d", "apy_90d_median", "apy_30d_mean", "suppressed"]`
(`models/seam.py:56`). The type has **no `spot` member**. The source comment states why:
the rule forbidding a spot rate is cheaper to enforce in the type than to police in
review, so rendering a spot reading is not merely discouraged but unrepresentable in this
field.

`display_apy_bps` is zero in two distinct situations, which a caller must not conflate:
the seam is suppressed (`display_apy_basis == "suppressed"`), or the true rate is under
half a basis point (`rounds_to_zero_bps == true`).

### 2.3 Capacity and allocation

| Field | Type | Notes |
|---|---|---|
| `tvl_usd` | `float`, >= 0 | Venue-wide capacity context, **not** what LODZ deployed |
| `allocation_bps` | `int`, 0..10000 | Share of routed BTC under the requested stope. A source leaves this at 0; the router policy fills it |
| `deployed_btc` | `float`, >= 0 | BTC LODZ currently holds here. Zero before the vault program is deployed |

### 2.4 Emissions

| Field | Type | Notes |
|---|---|---|
| `emission_token` | `Optional[str]` | Incentive token symbol when `yield_type == emissions` |
| `emission_ends_at` | `Optional[datetime]` | Published end of the programme |
| `emission_schedule` | `Optional[str]` | Free-form schedule description |

`emission_ends_at` null means no committed end date was published. The field description
states the distinction explicitly: that is **not** the same as no end.

### 2.5 Divergence loss

| Field | Type | Notes |
|---|---|---|
| `il_estimate_bps` | `Optional[int]` | Annualised divergence loss, positive, to subtract from the rate |
| `il_estimate_pct` | `Optional[float]` | The same, exact |
| `il_unknown` | `bool` | True when this is an LP position and no estimate could be produced |
| `il_model` | `Optional[str]` | How the estimate was produced |
| `pair_volatility_class` | `Optional[PairVolatilityClass]` | `correlated`, `mixed` or `uncorrelated`. Null for non-LP |
| `net_of_il_bps` | `Optional[int]` | `display_apy_bps` minus `il_estimate_bps`, floored at zero |
| `net_of_il_pct` | `Optional[float]` | The same, exact |

`net_of_il_*` is null when the divergence loss is unknown. The field description gives the
reason directly: a net figure computed against an unknown deduction would be fiction.

### 2.6 Gates

| Field | Type | Notes |
|---|---|---|
| `below_liquidity_floor` | `bool` | Venue TVL under the configured floor |
| `source_divergence` | `bool` | Two independent sources disagree beyond tolerance |
| `divergence_detail` | `Optional[str]` | Both figures and the measured gap |
| `routable` | `bool` | False when a gate excluded the seam from allocation |
| `exclusion_reason` | `Optional[str]` | Why |

A gated seam is **still returned** in the response. The field description states the
principle: hiding it would hide the reason.

### 2.7 Provenance

| Field | Type | Notes |
|---|---|---|
| `source_url` | `str` | Endpoint the rate was read from |
| `updated_at` | `datetime` | When the record was refreshed |
| `pool_address` | `Optional[str]` | On-chain pool or reserve account |
| `defillama_pool_id` | `Optional[str]` | Pool id used. Null when the seam aggregates several |
| `cross_check_source` | `Optional[str]` | Second independent source |
| `cross_check_apy_bps` | `Optional[int]` | What that source reported |

### 2.8 Asset trust, carried on every seam

| Field | Type |
|---|---|
| `trust_model` | `Optional[TrustModel]` -- `custodial`, `bridged`, `program-controlled` |
| `wrap_hops` | `Optional[int]` |
| `freezable` | `Optional[bool]` |
| `por_type` | `Optional[PorType]` -- `A` through `E` |
| `risk_tier` | `RiskTier` -- `low`, `medium`, `high` |

These are duplicated onto every seam rather than left to a join. The module docstring
states the intent: a rate must not be able to travel without its trust context, including
to a caller that only wanted the number.

### 2.9 Composition

| Field | Type | Notes |
|---|---|---|
| `attached_to` | `Optional[str]` | Id of the capital-bearing seam this stream rides on |
| `description` | `str` | One line on what earns the yield |
| `caveat` | `Optional[str]` | The thing a reader would be misled by if omitted |

Some venues pay two streams on one deposit: an organic stream and an incentive stream.
Those are modelled as **two seams**, so the split is never lost inside a blended figure. A
capital-bearing seam has `attached_to` null and holds an `allocation_bps` share; an
attached seam mirrors that allocation and deploys no extra capital.

The summation rule follows: **summing capital filters on `attached_to is None`; summing
yield counts both.** Getting this backwards double-counts principal.

There are no attached seams in the catalogue today because Solana BTC currently has zero
emissions programmes. The mechanism exists anyway, so that the moment one appears it
cannot be folded into the sustainable number.

`SeamDetailResponse` exposes the relationship in both directions: `seam` is the requested
position, `attached` lists the streams riding on it, and `base` is the capital bearing seam
this one rides on when the requested seam is itself attached. Exactly one of `attached` and
`base` is populated for any seam that participates in a composition.

### 2.10 Aggregates

`SeamTotals`, returned on every list response. All 27 fields:

| Field | Type | Notes |
|---|---|---|
| `seam_count` | `int` | Seams in the catalogue |
| `capital_bearing_count` | `int` | Seams with `attached_to is None` |
| `routable_count` | `int` | Seams that passed every gate |
| `sustainable_count` | `int` | Count by kind |
| `emissions_count` | `int` | Count by kind |
| `counterparty_count` | `int` | Count by kind |
| `sustainable_apy_bps` | `int` | Allocation weighted rate from streams outside users pay for |
| `sustainable_apy_pct` | `float` | The same, exact |
| `emissions_apy_bps` | `int` | Allocation weighted rate from incentive token programmes |
| `emissions_apy_pct` | `float` | The same, exact |
| `counterparty_apy_bps` | `int` | Allocation weighted rate out of other traders' losses |
| `counterparty_apy_pct` | `float` | The same, exact |
| `blended_apy_bps` | `int` | The three streams above added together |
| `blended_apy_pct` | `float` | The same, exact |
| `sustainable_share_bps` | `int` | Share of the blended rate, by kind |
| `emissions_share_bps` | `int` | Share of the blended rate, by kind |
| `counterparty_share_bps` | `int` | Share of the blended rate, by kind |
| `emission_exposure_bps` | `int` | Share of the blended rate depending on incentive programmes continuing |
| `il_estimate_bps` | `int` | Allocation weighted divergence loss across LP seams |
| `il_estimate_pct` | `float` | The same, exact |
| `net_of_il_bps` | `int` | Blended rate after subtracting the divergence loss estimate |
| `net_of_il_pct` | `float` | The same, exact |
| `il_unknown` | `bool` | True when any **allocated** LP seam has no estimate |
| `il_unknown_seam_ids` | `List[str]` | Which ones |
| `deployed_btc` | `float` | BTC currently routed |
| `catalog_tvl_usd` | `float` | Sum of venue-side TVL across capital bearing seams |
| `routable_tvl_usd` | `float` | The same, restricted to routable seams |

Note that the three per-kind rates are summed into `blended_apy_bps` but the components
remain individually addressable. A caller may present the blend, and may not present it
without the breakdown being available in the same payload.

Two fields carry a stated guarantee. `counterparty_apy_bps` is documented as zero for the
conservative and balanced stopes, which admit no counterparty exposure at all, and as
bounded by 3000 bps for the aggressive one. It used to be documented as zero everywhere,
on the reasoning that the router never allocated there. That was a description of the
router's current behaviour dressed as a property of the field, and it stopped being true
the moment the ceiling became per-profile: a guarantee that rests on nobody having
exercised a path is not a guarantee.
`emission_exposure_bps` is documented as always returned, **including when it is zero** --
the field never disappears just because the answer is nothing, since an absent field and a
measured zero read identically to a caller otherwise.

---

## 3. Yield classification

`YieldKind = Literal["sustainable", "emissions", "counterparty"]` (`models/common.py:25`).
Three kinds, not two. The three are **never summed into one another** at any layer.

| Kind | Who pays | Detection |
|---|---|---|
| `sustainable` | An outside user | Rate arises from `apyBase`: swap fees on an LP position, borrow interest on a lending reserve. `apyReward` absent or zero |
| `emissions` | The protocol prints it | `apyReward > 0` on the DefiLlama pool record, with `rewardTokens` naming the mint |
| `counterparty` | Another trader loses it | Venue mechanism, not a field: a vault whose income is the loss side of leveraged traders |

`sustainable` and `emissions` are separable from the pool record because DefiLlama splits
`apyBase` from `apyReward`. `counterparty` is not detectable from a rate field at all --
its signature is the venue's mechanism, so it is assigned per seam definition rather than
inferred.

The third kind exists for a specific reason recorded in the source comment: the highest
advertised BTC yield on Solana is a GMX-style vault paying 214.83 percent out of trader
losses. Printing that beside swap-fee income under one `sustainable` heading is the exact
deception this API exists to stop. It resembles sustainable yield in that outside money
arrives, and differs in that the payer must keep losing for it to continue, and the sign
flips when they do not.

The on-chain program carries all three variants, `Sustainable`, `Emissions` and
`Counterparty`
(`packages/anchor-program/programs/lodz-vault/src/state/mod.rs:154-180`). This was not
always true: the chain held two variants until 2026-08-16, so a counterparty seam could
be classified by the catalogue and had nowhere to be recorded on chain. The ledger of
where yield comes from is the product, and a ledger that has to file trader losses under
one of the other two kinds is stating something untrue.

Recording a kind and routing capital into it are separate questions. The chain now does
both, and answers them differently: it will record counterparty yield against any stope,
and it will only let capital sit on a counterparty seam in the forward chamber.
`RiskProfile::max_counterparty_bps()` is 0 for conservative, 0 for balanced and 3000 for
aggressive, enforced on registration and on every reallocation
(`state/mod.rs:266-272`). See `risk-spec.md` sections 2.4 and 7.5.

---

## 4. Collection pipeline

Source: `defillama`. Live status from `GET /health/detailed`:

```
"seam_source_health": {
  "source": "defillama", "healthy": true, "live": true, "seam_count": 16,
  "detail": "Live venue state from DefiLlama yields, cross-checked against Orca and
             Kamino's own APIs, with ninety day medians computed from per pool history."
}
```

### 4.1 Stages

```
[1] FETCH        https://yields.llama.fi/pools              primary pool records
                 https://yields.llama.fi/chart/{pool_id}    per-pool history, up to 647 points

[2] NORMALISE    resolve asset by mint against the verified table
                 reject denylisted mints fail-closed (assert_routable)
                 compute apy_7d / apy_30d_mean / apy_90d_median from history

[3] CROSS-CHECK  https://api.orca.so/v2/solana/pools                     for Orca seams
                 https://api.kamino.finance/kamino-market/{addr}/reserves/metrics
                                                                          for Kamino seams

[4] DIVERGENCE   estimate loss from pool price history, or mark il_unknown

[5] GATE         display window selection, liquidity floor, source divergence

[6] ALLOCATE     stope policy weights, renormalised across seams that passed

[7] PUBLISH      GET /seams, GET /metrics/header, GET /assay, GET /headlamp/risk
```

Live upstream list from the response provenance:

```
https://yields.llama.fi/pools
https://api.orca.so/v2/solana/pools
https://api.kamino.finance/kamino-market/7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF/reserves/metrics
https://yields.llama.fi/chart/{pool_id}
```

`provenance.mode` is one of `live_source` (venue state was read), `seed_catalog` (compiled
fallback) or `degraded_fallback` (a live read failed and compiled values were served).
The mode at capture time was `live_source`.

### 4.2 Cross-check and the 20 percent rule

Implemented in `display_rules._check_divergence`. The comparison is **relative**, against
the larger of the two readings:

```
scale   = max(|primary|, |other|)
gap_bps = round(|primary - other| / scale * 10_000)
diverged = gap_bps > settings.source_divergence_bps
```

Live threshold: `source_divergence_bps = 2000.0`, that is a **20 percent relative gap**.
On divergence the seam is suppressed -- `display_apy_pct` is forced to 0.0,
`display_apy_basis` becomes `suppressed`, and `routable` becomes false. It is still
returned, with `divergence_detail` carrying both figures and the measured gap.

The justification is that when two independent sources disagree materially, one of them is
stale and there is no way to tell which. Showing either as settled is a guess presented as
a measurement.

A second threshold prevents the rule from firing on noise. `_DIVERGENCE_FLOOR_PCT = 0.25`:
below a quarter of a percent, a relative gap carries no information, because 0.00459
percent against 0.00461 percent is a 0.4 percent relative difference between two numbers
that are both effectively zero. Below the floor, divergence is not evaluated, and the
detail string records that fact rather than silently skipping.

Seams currently carrying a cross-check source: the five Orca positions against
`api.orca.so`, and the three Kamino Lend reserves against `api.kamino.finance`. The rest
have a single source, which the response makes visible through a null `cross_check_source`
rather than implying corroboration that does not exist.

---

## 5. Display rules

Live thresholds from `GET /health/detailed`, produced by `display_rules.rule_summary()`:

```json
{
  "liquidity_floor_usd": 100000.0,
  "source_divergence_bps": 2000.0,
  "spot_apy_sanity_ceiling_bps": 1000000.0,
  "divergence_comparison_floor_pct": 0.25
}
```

Every rule is applied **server-side**, so no caller can opt out.

### 5.1 Never render a spot rate

Basis: the Orca cbBTC-USDC pool printed **74,187 percent** `apyBase` on one day of its 646
day history, a low-TVL calculation artifact. Source:
`https://yields.llama.fi/chart/2651188f-6b05-473e-9cfb-977a4ad094ba`.

`_select_display` picks the freshest robust window: `apy_7d`, then `apy_90d_median`, then
`apy_30d_mean`. A candidate is skipped if it is null, negative, or above
`spot_apy_sanity_ceiling_bps` (1,000,000 bps = 10,000 percent), on the reasoning that a
**window average** above that ceiling is itself an artifact rather than a rate. If no
candidate survives, the basis is `suppressed` and the rate is zero.

The ninety day figure is a **median, not a mean**, precisely so a single artifact day
cannot move it.

### 5.2 A rate below the liquidity floor is not a seam

Basis: Zeus Bitcoin Market USDC advertised **104.6 percent** against **$10,927** of
capacity. The rate was real; there was nowhere to put money.

Live floor: `liquidity_floor_usd = 100000.0`. Below it, `below_liquidity_floor` is set,
the seam is excluded from allocation, and it is still returned with the reason. No seam in
the current catalogue is below the floor; the lowest is `orca-wbtc-usdc-lp` at $106,503.

### 5.3 Divergence loss must be shown or declared missing

Basis: DefiLlama returns `il7d = null` for **every** BTC pool on Solana. Every published LP
rate in this space is gross of divergence loss.

For an LP seam the service either produces its own estimate or sets `il_unknown = true` and
leaves `net_of_il_*` null. For a lending reserve or a vault share, `il_estimate_pct` is
0.0 with `il_model = "not_applicable_single_sided"` -- a single-sided position has no
second leg to diverge from, so zero here is a fact about the position rather than a
missing measurement. The distinction between "zero" and "unknown" is preserved in the
schema instead of collapsing both to a blank.

### 5.4 Sub-basis-point rates must survive

Basis: Kamino cbBTC pays 0.00459 percent. Covered in section 1.

### 5.5 Points programmes are not converted to a rate

Unissued points have no price. Assigning one would move an emissions expectation into a
sustainable number. LODZ does not convert points to APY; where a venue runs a programme,
that yield is **absent from these figures rather than estimated into them**. The
consequence is that reported yield can understate what a depositor eventually receives,
and understating is the direction this protocol errs in.

### 5.6 Allocation ceiling relative to venue TVL -- specified, NOT yet enforced

The research rule states that LODZ capital in any seam should not exceed 10 percent of
that seam's TVL, because a position that dominates a pool does not realise the displayed
rate.

**Status: not implemented.** `seam_service.apply_allocation` applies the stope policy
weights and renormalises across seams that passed the gates; it does not compare an
allocation against `tvl_usd`. `tvl_usd` is read only for reporting
(`seam_service.py:281-282`). No `allocation_notes` entry mentions a capacity cap, and the
live response carries an empty `allocation_notes` array.

This gate has no effect while `deployed_btc` is 0.0 across the catalogue and the vault
program is not deployed, since no capital is routed. It must be enforced **before** the
first deposit. The check belongs in `apply_allocation`: convert the effective
`allocation_bps` into a USD figure against total deposits, compare against
`0.10 * seam.tvl_usd`, and reduce the weight with an `allocation_notes` entry stating the
reduction, using the same pattern the function already uses for gate-excluded seams.

Recording this as an open gap is deliberate. A specification that describes an unenforced
rule as active is worse than one that names the gap.

---

## 6. Emissions are zero, and that is a measurement

Live totals at capture: `emissions_count = 0`, `emissions_apy_pct = 0.0`,
`emission_exposure_bps = 0`.

The verification that this is measured rather than missing:

| Check | Result |
|---|---|
| BTC pools examined | 94, full enumeration |
| Pools with `apyReward > 0` | 0 |
| History depth on the two largest | 647 daily points, no reward on any day |
| Control: non-BTC Solana pools with `apyReward > 0` in the same snapshot | **15** |

The control is what makes the zero load-bearing. Fifteen non-BTC Solana pools in the same
DefiLlama snapshot report reward APY, so the field is populated and collection is working.
BTC pools simply have none. Source: `https://yields.llama.fi/pools`.

One qualifier is attached and must not be dropped: this is **zero token emissions**. Points
programme sizes remain unverified because no unauthenticated public API for them was found,
so a venue running one contributes yield that is absent from these figures. The claim is
"no BTC pool on Solana currently pays a reward token", not "no incentive of any form
exists".

This zero is treated as product content rather than an empty state. Competing BTC yield
products on other chains reach double digits substantially through emissions; explaining
why this catalogue's emissions column reads zero is itself the demonstration.

---

## 7. Current catalogue

Captured from `GET /seams?stope=balanced` at `2026-08-15T07:54Z`. Sixteen seams, fifteen
routable. Rates are `display_apy_pct` unless noted; every basis is `apy_7d`.

| Seam id | Venue | Asset | Kind | Yield kind | Spot % | Display % | TVL USD | Alloc bps | Routable | IL bps |
|---|---|---|---|---|---|---|---|---|---|---|
| `orca-cbbtc-usdc-lp` | Orca | cbBTC | lp | sustainable | 14.33292 | 15.47047 | 6,321,719 | 2500 | O | 65 |
| `kamino-cbbtc-lend` | Kamino Lend | cbBTC | lending | sustainable | 0.00459 | 0.00475 | 44,065,051 | 1500 | O | 0 |
| `orca-cbbtc-wbtc-lp` | Orca | cbBTC | lp | sustainable | 5.86054 | 4.27787 | 1,011,939 | 1500 | O | 0 |
| `orca-sol-cbbtc-lp` | Orca | cbBTC | lp | sustainable | 15.49729 | 21.83382 | 4,584,863 | 1500 | O | 34 |
| `orca-usdg-xbtc-lp` | Orca | xBTC | lp | sustainable | 1.28415 | 1.53909 | 2,012,176 | 1000 | O | 66 |
| `save-cbbtc-lend` | Save | cbBTC | lending | sustainable | 0.19000 | 0.19000 | 1,786,910 | 1000 | O | 0 |
| `kamino-usdg-xbtc-lp` | Kamino Liquidity | xBTC | lp | sustainable | 1.02620 | 1.22628 | 2,012,670 | 500 | O | 66 |
| `loopscale-zbtc-lend` | Loopscale | zBTC | lending | sustainable | 1.05838 | 1.05804 | 294,543 | 500 | O | 0 |
| `gmtrade-btc-usdc-vault` | GMTrade | BTC-USDC | perp_vault | **counterparty** | 214.82797 | 180.32895 | 1,709,208 | 0 | **X** | 0 |
| `orca-sol-wbtc-lp` | Orca | WBTC | lp | sustainable | 7.19710 | 10.97604 | 930,375 | 0 | O | 37 |
| `orca-cbbtc-jlp-lp` | Orca | cbBTC | lp | sustainable | 4.40166 | 6.64159 | 213,832 | 0 | O | **unknown** |
| `orca-wbtc-usdc-lp` | Orca | WBTC | lp | sustainable | 2.12669 | 4.66426 | 106,503 | 0 | O | **unknown** |
| `orca-cbbtc-xbtc-lp` | Orca | cbBTC | lp | sustainable | 0.89634 | 0.79801 | 277,698 | 0 | O | **unknown** |
| `kamino-wbtc-lend` | Kamino Lend | WBTC | lending | sustainable | 0.02462 | 0.02449 | 153,505 | 0 | O | 0 |
| `kamino-xbtc-lend` | Kamino Lend | xBTC | lending | sustainable | 0.00063 | 0.00077 | 13,595,308 | 0 | O | 0 |
| `jupiter-lend-btc` | Jupiter Lend | cbBTC and others | lending | sustainable | 0.00000 | 0.00000 | 5,984,906 | 0 | O | 0 |

Each seam's `source_url` is its DefiLlama chart endpoint,
`https://yields.llama.fi/chart/{pool_id}`, except `jupiter-lend-btc` which aggregates
several markets and reads `https://yields.llama.fi/pools`. Aggregate seams carry a null
`defillama_pool_id` and a null `asset_mint` by design, since neither has a single value.

Portfolio aggregates for the balanced stope at capture:

| Metric | Value |
|---|---|
| `sustainable_apy_pct` | 8.072209 |
| `emissions_apy_pct` | 0.0 |
| `counterparty_apy_pct` | 0.0 |
| `blended_apy_pct` | 8.072209 |
| `il_estimate_pct` | 0.312681 |
| `net_of_il_pct` | 7.759528 |
| `il_unknown` | false |
| `emission_exposure_bps` | 0 |
| `catalog_tvl_usd` | 85,061,206 |
| `routable_tvl_usd` | 83,351,998 |

`il_unknown` is false at portfolio level while three individual seams report unknown
divergence loss. This is consistent, not contradictory: the flag covers **allocated** LP
seams, and all three unknowns carry `allocation_bps = 0`. Were any of them allocated, the
flag would flip and the net figure would become a partial answer.

The GMTrade vault is the clearest illustration of the classification rule. At 180.33
percent display rate it is by a wide margin the highest number in the catalogue, and it is
`routable: false` with `allocation_bps: 0`. It appears in the response so that a reader can
see it was found, classified and declined, rather than quietly omitted.

---

## 8. Excluded candidates

Returned in `excluded_candidates` on every seam response. An absence with an explanation
rather than a gap.

| Candidate | Reason |
|---|---|
| `drift-btc-perp-basis` | Excluded after on-chain inspection, not a data failure. Last funding update 2026-04-01T18:00:00Z; all 200 sampled transactions in the following week failed |
| `zeus-bitcoin-market-usdc` | 104.6 percent against $10,927 of supply. The rate is real and there is nowhere to put money |
| `kamino-fbtc-lend` | $5.48M supplied, zero borrowed, zero paid. Deposits without a market |
| `zeta-adrena-flashtrade-basis` | Unverified. Every unauthenticated API failed |

### 8.1 Basis seams are not supported

Stated explicitly rather than omitted: **LODZ does not currently support basis seams.**

The reason is not that basis trades are unattractive. It is that no verifiable data path
to them exists. `data.api.drift.trade` returns 403 and `api.zeta.markets/markets` returns
403, so neither can be read without authentication. For Drift, an on-chain check was run in
place of the API and it disqualified the venue on its own evidence: funding has not updated
since 2026-04-01 and a 200-transaction sample from the following week failed in full. Open
interest of 250 BTC did not make it a live market.

For Zeta, Adrena and Flash Trade the position is weaker still: the same on-chain survival
check has not been run, so their status is **unverified**, not "excluded on evidence".

Supporting a basis seam requires on-chain parsing of a live perpetuals market, including a
liveness check of the kind applied to Drift. Until that exists, no basis figure appears in
any LODZ interface. Saying "not supported yet" is more honest than dropping the category
without comment, because a reader who knows basis trades exist would otherwise assume they
were considered and found unremarkable.

---

## 9. Divergence loss methodology

Implemented in `apps/service/src/services/divergence.py`.
`MODEL_NAME = "constant_product_realized_window_annualized"`.

### 9.1 The model

For a two-sided position at price ratio `r` relative to entry, holding the pair in a pool
rather than in a wallet leaves:

```
value_ratio = 2 * sqrt(r) / (1 + r)
```

This is at most 1 at `r = 1` and falls as the legs move apart in either direction. The
shortfall `1 - value_ratio` is the divergence loss. It is measured over the observed price
window and scaled to a year so it is comparable against an annualised fee rate.

### 9.2 What the estimate is not

Both limits are reported through `il_model` and the seam caveat rather than buried:

- It is a **constant product** formula. Concentrated positions held in a narrow range
  experience more divergence than this, not less. The estimate is therefore a **floor**,
  not a forecast. Every LP seam in this catalogue is a concentrated position.
- Annualising a short observed window assumes the same drift continues. A calm week
  understates and a violent week overstates.

### 9.3 Guards and the unknown path

`_MAX_PLAUSIBLE_RATIO = 10.0`: divergence implying a 10x price ratio move inside a short
window is treated as a data problem rather than a measurement, on the stated reasoning
that such a move is a broken feed far more often than a market. Price inputs are filtered
to finite positive floats before use.

When no usable price history exists the estimate is `None`, the seam is marked
`il_unknown = true`, and `net_of_il_*` stays null. Three seams are in this state today:
`orca-cbbtc-jlp-lp`, `orca-wbtc-usdc-lp` and `orca-cbbtc-xbtc-lp`.

The governing principle is stated in the module docstring and is worth repeating as a
rule: **a missing estimate is visible, and an invented one would not be.** A caller
receiving `il_unknown` must present the rate as incomplete rather than net.

---

## 10. Caller obligations

A conforming client must:

1. Render `display_apy_pct`, never `apy_pct`. The spot field exists for completeness.
2. Render the percentage rather than `0` when `rounds_to_zero_bps` is true.
3. Present the rate as incomplete, not net, when `il_unknown` is true.
4. Show `yield_type` alongside any rate, and never sum the three kinds into one figure
   without the breakdown remaining available.
5. Surface `exclusion_reason` for a seam with `routable: false` rather than filtering it out.
6. Resolve assets by `asset_mint`, never by `asset`.
7. Carry `trust_model`, `wrap_hops`, `freezable` and `por_type` wherever the rate is shown.

---

## 11. Unverified

| Item | Status | What would settle it |
|---|---|---|
| Points programme sizes | Unverified | No unauthenticated public API found. Until then "zero emissions" carries the "token emissions" qualifier |
| Zeta, Adrena, Flash Trade perp liveness | Unverified | Apply the on-chain survival check used on Drift |
| Drift `Custom: 101` error meaning | Unverified | Drift error code table. Decides halted market against stale oracle |
| BTC funding rate levels and sign bias | Unverified | Requires one live BTC perpetuals market first |
| Jupiter Perps (JLP) yield structure | Unverified | Needs a working `perps-api.jup.ag` path or on-chain custody parsing |
| Meteora BTC pools | Unverified | Official API paths return 404. DexScreener shows a WBTC/cbBTC pool at $291,659; an alternative route is needed |
| Concentrated-range correction to the divergence model | Not implemented | Present estimate is a constant-product floor for every LP seam |
| Allocation ceiling at 10 percent of venue TVL | **Specified, not enforced** | See section 5.6. Required before the first deposit |

The USD reference deserves the same treatment. `usd_reference` at capture reports
`btc_usd = 100000.0` with `source = "operator-configured reference (BTC_USD_REFERENCE)"`
and `live = false`. Every USD figure rendered beside a BTC amount inherits that constant.
The schema carries the `live` flag precisely so a reference price is never presented as a
market quote.
