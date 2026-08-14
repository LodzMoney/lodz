# LODZ

Yield from the bedrock.

A BTC yield layer on Solana that routes wrapped BTC into yield seams and reports
where every basis point actually came from.

The interesting part is not the routing. It is the attribution.

[lodz.money](https://lodz.money) &middot; [LodzMoney](https://github.com/LodzMoney) &middot;
[lodz-sdk](https://github.com/LodzMoney/lodz-sdk)

| Repository | Contents |
|---|---|
| **lodz** (this one) | Anchor vault program, IDL, and the specifications it enforces |
| [lodz-sdk](https://github.com/LodzMoney/lodz-sdk) | Attribution engine, risk model, redemption queue, seam router, SDK and CLI |

---

## Why this exists

We measured the Solana BTC yield landscape on 2026-08-15 before writing the program,
using public endpoints only. Four results changed the design.

### 1. BTC lending on Solana pays approximately nothing

| Venue | Asset | Supply APY |
|---|---|---|
| Kamino | cbBTC | 0.00459% |
| Kamino | xBTC | 0.00063% |
| Kamino | FBTC | 0% |
| Jupiter Lend | all BTC markets | 0% |
| Loopscale | zBTC | 1.06% |

About $75.4M of BTC sits in these lending markets. It earns nothing because nobody
borrows against it: cbBTC utilisation is 3.2%. Interest is paid by borrowers, and
there are no borrowers.

The only exception is Loopscale zBTC at 1.06%, on $295K of capacity.

So "deposit BTC, earn lending interest" is not a seam that exists on Solana today.
A protocol advertising an APY for it is quoting a number the chain does not produce.

### 2. The real yield is liquidity provision fees, and it is not what it looks like

| Pool | APY (fees) | TVL |
|---|---|---|
| Orca cbBTC-USDC | 14.996% | $6.32M |
| Orca SOL-cbBTC | 16.046% | $4.58M |

These are genuine. Traders pay them. But the figure is fee revenue before impermanent
loss, and DefiLlama reports `il7d` as null for every one of these pools, so the number
you see quoted elsewhere is a gross figure presented as a net one.

We compute an impermanent loss estimate and show it. Where we cannot compute one, the
seam is labelled `ilUnknown`. We do not invent the estimate.

### 3. Token emissions on Solana BTC pools are currently zero

We checked all 94 BTC-related pools across 647 days of history. Not one shows a
non-zero reward APY.

This is a measurement, not a gap in our data collection: the same snapshot finds 15
pools with `apyReward > 0` elsewhere on Solana. The collector works. The number is zero.

That zero is the most useful thing on our dashboard. When a competitor advertises a
double-digit BTC yield, one of three things is true:

- fee revenue is shown without impermanent loss deducted, or
- a points programme has been converted into an APY, or
- leverage is folded into the headline.

Separating the three is the entire product.

### 4. There is a third kind of yield, and it is not sustainable or emitted

GMTrade's BTC-USDC vault reported 214.828% on $1.71M. The source of that yield is
trader losses.

It behaves like fee revenue on a chart and is nothing like it in character. Putting it
in the same column as exchange fees, in the same colour, misleads the reader. So we
give it its own kind.

```
sustainable    trading fees, borrow interest -- money an outside user actually paid
emissions      protocol token emissions -- money the issuer printed
counterparty   the other side's losses -- money a trader lost
```

Most attribution models have two categories. The measurements say there are three.

---

## Assets

Identified by mint, never by symbol. WBTC has two distinct mints on Solana and a
symbol lookup silently picks the wrong one.

| Asset | Mint | Trust model | Wrap hops | Freezable |
|---|---|---|---|---|
| cbBTC | `cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij` | custodial (Coinbase) | 1 | yes |
| WBTC | `3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh` | bridged (BitGo to Ethereum to Wormhole) | 2 | no |
| zBTC | `zBTCug3er3tLyffELcvDNrKkCymbPWysGcWihESYfLg` | program-controlled (Zeus, PDA) | 1 | no |
| xBTC | `CtzPWv73Sn1dMGVU3ZtLv9yWSyUAanBni19YWDaznnkn` | custodial (OKX) | 1 | yes |

cbBTC, WBTC and zBTC each represent a different trust model: a custodian holds it, a
bridge holds it, or a program holds it. Supporting all three is what makes a risk
breakdown meaningful rather than decorative.

These are wrapped representations of bitcoin. They are not bitcoin, and this codebase
does not describe them as such. Every one carries custody or bridge risk that the
underlying chain does not.

### Denylist

Blocked by mint:

```
9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E   soBTC (Sollet)   depegged -99.96%
21BTCo9hWHjGYYUQQLqjLgDBxjcn8vDt4Zic7TB3UbNE   21BTC            economically dead
5XZw2LKTyrfvfiskJ78AMpackRjPcyCif1WhUsPDuVqQ   WBTC (BitGo canonical)  $46K liquidity on Solana
```

---

## Display rules

These are enforced in the program and in the API, not left to the interface.

| Rule | Why |
|---|---|
| No spot APY. Seven-day value or ninety-day median only | Orca cbBTC-USDC history contains a day at 74,187% apyBase, an artefact of low TVL |
| Liquidity seams carry an impermanent loss estimate, or are marked `ilUnknown` | DefiLlama `il7d` is null for every relevant pool |
| `counterparty` yield gets its own label and colour | 214% next to exchange fees reads as the same thing and is not |
| Two sources disagreeing by more than 20 percent hides the seam and raises a flag | Stale data must not be presented as settled |
| TVL floor of $100K | A 104.6% quote on $10,927 of capacity does not survive an allocation |
| Points programmes are never converted to an APY | Attaching a price to unissued points relabels emissions as fee revenue |

One further rule is specified and **not yet enforced in code**: no allocation may exceed
10 percent of a seam's TVL, because our own capital moving a pool means the quoted rate
is not the realised rate. The router currently applies its policy weights and
redistributes the share of any seam a gate rejected; it does not yet compare an
allocation against venue TVL. The check is inert today because nothing is deployed and
no capital is routed, and it has to land before the first deposit. It is listed here
rather than omitted, because a specification that reads as enforced when it is not is
worse than one that names the gap. See `docs/seam-spec.md` section 5.6.

Basis seams are absent from the API and the interface. Drift and Zeta both return 403
without authentication, and there is no unauthenticated data path, so the seam is
unsupported until on-chain parsing is implemented. It is not stubbed and not estimated.

---

## Data sources

Every endpoint below was confirmed returning 200 at time of measurement.

```
https://yields.llama.fi/pools
https://yields.llama.fi/chart/{poolId}
https://api.kamino.finance/kamino-market/{market}/reserves/metrics
https://api.orca.so/v2/solana/pools
https://api.solend.fi/v1/reserves?scope=all
```

Confirmed failing, and therefore not wired in: `data.api.drift.trade` (403),
`api.zeta.markets/markets` (403).

---

## Layout

```
programs/lodz-vault/     Anchor program
  src/state/             adit, seam, stope, orecart, keeper, miner, config
  src/instructions/      deposit, accrual, redemption, keeper, admin
  src/math.rs            fixed point arithmetic
  src/errors.rs
  src/events.rs
tests/                   integration tests against a local validator
scripts/                 instruction discriminator helper
idl/lodz_vault.json      generated IDL, checked in so clients can build without Anchor
docs/risk-spec.md        risk layers, asset table, on-chain enforcement, incident record
docs/seam-spec.md        seam schema, yield classification, display rules, IL methodology
```

The specifications cite the source lines that enforce them, so a claim in `docs/` can be
checked against `programs/lodz-vault/src/` without taking either on trust.

- `adit` is the deposit entry point.
- `seam` is a yield source with a kind, a rate and a risk tier.
- `stope` is a risk-tiered vault.
- `orecart` is the redemption queue.
- Keepers post a bond and are slashed for reporting a rate that disagrees with the seam.

---

## Build

```
anchor build
anchor test
```

Rust 1.79 or newer, Anchor 0.31, Solana 1.18 or newer.

`anchor test` starts a local validator and exercises the full deposit, accrual and
redemption cycle. No network is contacted.

---

## Status

The program builds and the local test suite passes. It is not deployed to mainnet, has
not been audited, and the redemption queue has not been exercised under load.

Principal is recovered one-for-one through the redemption queue. The queue can be
delayed. Nothing here removes custody risk, bridge risk, smart contract risk or the
impermanent loss that liquidity provision carries. Yield figures are measurements of
what happened, not predictions of what will happen.

---

## Licence

MIT. See LICENSE.
