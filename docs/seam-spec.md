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

Two fields carry a stated guarantee. `counterparty_apy_bps` is documented as zero under
every stope because the router does not allocate to counterparty seams.
`emission_exposure_bps` is documented as always returned, **including when it is zero** --
the field never disappears just because the answer is nothing, since an absent field and a
measured zero read identically to a caller otherwise.
