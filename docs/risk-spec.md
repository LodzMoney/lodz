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
