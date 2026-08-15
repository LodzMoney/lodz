# LODZ Architecture

Version 1. Last reviewed 2026-08-15.

This document describes how LODZ is put together: which repositories exist and why they
are separate, where a rule is enforced versus merely reported, how data moves from a
public venue API to a rendered figure, and what is deployed today. It is descriptive of
the system as built. Where a structure exists because of a measured failure, the entry
says which failure.

Companion documents: `risk-spec.md` (risk measurement and disclosure), `seam-spec.md`
(yield classification and display rules), `security.md` (implementation hardening).

Confirmed identifiers used throughout:

| Item | Value |
|---|---|
| Domain | `lodz.money` |
| Public organisation | `LodzMoney` |
| Public repositories | `LodzMoney/lodz`, `LodzMoney/lodz-sdk` |
| Private repositories | `Cryptottat/lodz-web`, `Cryptottat/lodz-api`, `Cryptottat/lodz-core` |
| npm packages | `@lodz/cli`, `@lodz/sdk`, `@lodz/assay-engine`, `@lodz/headlamp-risk`, `@lodz/seam-router`, `@lodz/orecart-queue` |

---

## 1. Three repositories, not one

The working tree looks like a monorepo and is not one. There are three independent git
repositories inside it, and the project root is deliberately not a repository at all:

```
lodz/                      no git repository here, by design
├── apps/web/              -> Cryptottat/lodz-web     (Vercel)
├── apps/service/          -> Cryptottat/lodz-api     (Railway)
├── packages/              -> Cryptottat/lodz-core    (npm, Anchor program)
├── github/                staging for the public mirror
└── docs/                  specifications, mirrored to LodzMoney/lodz
```

Verified: `git -C apps/web remote get-url origin` returns `Cryptottat/lodz-web.git`,
`apps/service` returns `lodz-api.git`, `packages` returns `lodz-core.git`, and
`git rev-parse --git-dir` at the root finds nothing.

### 1.1 Why the root is not a repository

Vercel and Railway both clone a repository and treat its root as the build context. A
platform pointed at a repository whose root is the monorepo needs a Root Directory setting
to find the application, and that setting is the failure point.

The measured failure: setting Vercel's Root Directory to `apps/web` against a repository
whose root already *is* the web application produces a path that does not exist, and the
build fails in its entirety rather than partially. The two configurations are
indistinguishable from the dashboard, and the error surfaces as a missing directory rather
than as a misconfiguration.

Giving each deployable its own repository removes the setting. `Cryptottat/lodz-web` has
`package.json` at its root because it *is* the web app; `Cryptottat/lodz-api` has
`requirements.txt` at its root because it *is* the service. There is no Root Directory to
get wrong.

### 1.2 Why not git subtree

`git subtree` would let one repository carry the others as prefixed histories. It is not
used. Subtree splits rewrite commit identity on every push, so the history the platform
builds from is not the history that was reviewed, and a conflict during a split is
resolved by a tool rather than by a person. The three repositories are pushed
independently, and a script stages the public mirror explicitly rather than deriving it.

### 1.3 What the root does hold

The root `package.json` declares `workspaces: ["packages/*"]` and nothing else. It manages
the TypeScript packages for local development only. It is not published, not deployed and
not the parent of the applications.

---

## 2. The web application does not depend on the packages

`apps/web` has zero dependencies on any `@lodz/*` package. Verified: its `package.json`
lists no `@lodz/*` entry and no `workspace:` protocol dependency.

This is a constraint imposed by the repository split rather than a preference. Vercel
builds from `Cryptottat/lodz-web` alone. A `workspace:` dependency on
`packages/assay-engine` resolves locally and does not exist in the cloned repository, so
the build would pass on a developer machine and fail on the platform -- the worst shape a
build failure can take.

### 2.1 What replaces it

The service sends the web application a catalogue, and the web application does arithmetic
on it:

| Sent by the service | Computed in the browser |
|---|---|
| Per-seam rate, window basis, TVL | Projected yield for an entered BTC amount |
| Yield kind per seam (three-way) | Split of that projection across the three kinds |
| Divergence loss estimate, or its absence | Net-of-divergence figure, or a withheld one |
| Router allocation per profile | Per-seam contribution and share |
| Queue policy parameters | Estimated redemption wait for a size |

The division is: **the service owns the rules, the browser owns the multiplication.** A
rate, a classification and an allocation are decisions and are made once, server side. A
projection for 1.5 BTC is an arithmetic consequence of those decisions and needs no round
trip.

### 2.2 Why the boundary sits exactly there

The Assay Board is the product's first interaction. A user types an amount and expects the
decomposition to move as they type. A server round trip per keystroke would make that
interaction feel broken regardless of how fast the API is.

Putting the arithmetic in the browser achieves that. Putting the *rules* in the browser
would duplicate them, and duplicated business rules diverge -- which is how a marketing
page ends up quoting a number the API does not agree with. The catalogue is the contract:
the browser cannot invent a rate because it has none of its own, and it cannot reclassify
yield because the kind arrives already decided.

The same catalogue feeds the CLI and the SDK. Three clients, one set of rules, no
reimplementation.

---

## 3. Data flow

```
  DefiLlama yields      Orca v2 pools      Kamino reserves      Save reserves
  (rates, history)      (rate, prices)     (supply APY)         (supply APY)
         |                    |                   |                   |
         +--------------------+---------+---------+-------------------+
                                        |
                            [ 1 ] fetch, shape-check
                                        |
                            [ 2 ] normalise units
                                        |
                            [ 3 ] cross-check, 20% tolerance
                                        |
                            [ 4 ] display rules, gates
                                        |
                            [ 5 ] cache: TTL + stale-while-revalidate
                                        |
                            [ 6 ] allocate per stope, aggregate
                                        |
                +-----------------------+-----------------------+
                |                       |                       |
          apps/web (proxy)          @lodz/cli               @lodz/sdk
```

**[1] Fetch and shape-check.** Only endpoints verified reachable without authentication
are called. Endpoints that returned 403 or nothing during research are not attempted in
production, because retrying a closed API is a slow path to the same answer. A 200 is not
accepted as success: each response is checked for the fields it should carry, because one
surveyed endpoint returns 200 on every path with a domain-parking page.

**[2] Normalise units.** Sources disagree about representation, not only about value.
Kamino reports a supply rate as a fraction; DefiLlama reports the same quantity as a
percentage. Comparing them unconverted manufactures a 99 percent disagreement on every
reserve. Units are normalised before any comparison.

**[3] Cross-check.** Every rate readable from two independent sources is compared. A
relative gap beyond 20 percent suppresses the seam and flags it, because one side is stale
and there is no way to tell which from inside. Below a 0.25 percent absolute floor the
comparison is skipped: at rates measured in thousandths of a percent, a relative
difference carries no information.

**[4] Display rules.** Spot rates are never surfaced; a seven day window, a ninety day
median or a thirty day mean is chosen instead. Venues below a TVL floor are flagged and
excluded from allocation. Full detail in `seam-spec.md`.

**[5] Cache.** An in-memory TTL cache with stale-while-revalidate. Inside the TTL the
cached copy is returned. Past it, the cached copy is returned *immediately with a stale
flag* and a refresh runs behind the response. Past a longer bound the request blocks on a
real fetch, because at some age a fast answer stops being worth more than a correct one. A
background worker refreshes on an interval so a visitor rarely pays for a cold fetch; the
interval is unhurried because the primary upstream sits behind a cache measured at roughly
thirty minutes, and polling faster re-reads the same bytes.

**[6] Aggregate.** The router policy is applied per risk profile and the three yield kinds
are summed independently. A seam excluded by a gate is dropped from the allocation and the
remaining weights are renormalised, with the adjustment reported -- silently leaving that
share undeployed would misstate the blended rate.

### 3.1 Failure behaviour

The pipeline never substitutes a plausible number for a missing one.

| Condition | Response |
|---|---|
| Cached copy past TTL | Served with `stale: true` and an age in seconds |
| Live read fails, snapshot valid | Frozen snapshot, `mode: degraded_fallback`, `served_by: fallback_snapshot`, reason quoted |
| Live read fails, snapshot expired | HTTP 503. No data rather than old data |
| Divergence loss not computable | `il_unknown: true`, net figure withheld as null |

`served_by` names what actually produced the numbers, which is not always the configured
pipeline. Health reports that field, so a degraded service reads as degraded rather than
as the pipeline it was asked to use.

This design has a specific origin, recorded in `security.md` section 4: an early build
served a catalogue of plausible figures that were not measurements. The rule that came out
of it is that a fallback must announce itself, must expire, and must never be quieter than
the failure it is covering.

---

## 4. On-chain versus off-chain

The clearest way to read this system is to ask which claims survive without trusting LODZ.

### 4.1 Enforced by the program -- verifiable without trusting the operator

These are properties of deployed bytecode. Anyone can read the account state and confirm
them.

| Property | Mechanism |
|---|---|
| A redemption cannot be paid early | `require!(now >= orecart.claimable_at)`, `instructions/redemption.rs:365-368` |
| The delay is fixed at request time | `claimable_at` stamped once and stored, `redemption.rs:162, 241` |
| A ticket cannot be claimed twice | Status check against `TicketAlreadyClaimed`, `redemption.rs:360` |
| Only the owner can move a position | `has_one = owner`, `redemption.rs:64, 292, 300` |
| Only the authority can change parameters | `has_one = authority`, `admin.rs:161, 224, 328, 428, 562, 612` |
| Parameters have ceilings a key cannot raise | `MAX_FEE_BPS = 500` and three delay ceilings, `state/mod.rs:74-85` |
| A keeper has capital at risk | Bond and slash paths, `instructions/keeper.rs`, `admin.rs:655` |
| Arithmetic cannot wrap silently | `overflow-checks = true` plus checked forms |
| An unregistered mint cannot be deposited | Adit PDA derived from the mint itself |

### 4.2 Reported off-chain -- true only if the indexer is honest

These are measurements and estimates. The program does not and cannot verify them.

| Quantity | Why it cannot be on-chain |
|---|---|
| Venue APY | Lives in third-party APIs. No oracle publishes it. |
| Ninety day median, seven day window | Computed from off-chain history series |
| Divergence loss estimate | Derived from pool price history, and it is a model |
| Yield kind classification | A judgement about where money comes from |
| TVL and liquidity floors | Third-party aggregation |
| Cross-source agreement | Compares two off-chain sources |

The honest framing: **the queue is trustless, the yield figures are not.** A depositor does
not have to believe LODZ about when their principal becomes claimable -- that is in the
account. They do have to believe LODZ about what a venue is paying, which is why every
rate ships with its source URL, its window basis, its cross-check result and its
provenance mode. Auditability is the substitute for enforcement where enforcement is
impossible.

---

## 5. The Anchor program

Source: `packages/anchor-program/programs/lodz-vault/src/`. Interface:
`packages/anchor-program/target/idl/lodz_vault.json`.

Declared program id, from `lib.rs:70`:

```
F9XmBYVEyEwFyHAdMJs6uBvyRag3AFhQ6YMZvqm13SLW
```

This is a build-time identity, not a deployment. See section 6.

IDL totals, read from the file: **17 instructions, 8 accounts, 16 events, 53 errors, 31
types.**

### 5.1 Accounts and their PDA seeds

Seed constants are declared once in `state/mod.rs:44-62` and referenced everywhere; no
call site builds a seed from a literal.

| Account | Seeds | Cardinality |
|---|---|---|
| `VaultConfig` | `["vault_config"]` | One per program |
| `Adit` | `["adit", asset_mint]` | One per accepted asset |
| `Stope` | `["stope", stope_id_le]` | One per risk profile |
| `Seam` | `["seam", seam_id_le]` | One per venue position |
| `Miner` | `["miner", owner, stope_id_le]` | One per depositor per profile |
| `Orecart` | `["orecart", owner, ticket_index_le]` | One per redemption ticket |
| `OrecartQueue` | `["orecart_queue", stope_id_le]` | One per profile |
| `Keeper` | `["keeper", keeper_authority]` | One per keeper |

Two further PDAs hold tokens rather than state: `["adit_vault", asset_mint]` for deposited
collateral and `["bond_vault"]` for keeper bonds. Both have `VaultConfig` as their token
authority.

Every derivation is a constant plus a discriminating key or a little-endian id. Verified
against the call sites: `MINER_SEED` at `redemption.rs:62, 290`, `ORECART_QUEUE_SEED` at
`redemption.rs:70, 308`, `ORECART_SEED` at `redemption.rs:80, 298`, and the full
cross-instruction table in `security.md` section 5.3.

### 5.2 Instructions

All 17, grouped by who may call them. Names and arguments taken from the IDL.

**Administrative** -- authority only:

| Instruction | Arguments |
|---|---|
| `initialize_vault` | `params` |
| `initialize_bond_vault` | -- |
| `register_adit` | `params` |
| `open_stope` | `stope_id`, `risk_profile` |
| `register_seam` | `seam_id`, `stope_id`, `params` |
| `update_seam_allocation` | `seam_id`, `stope_id`, `new_allocation_bps` |
| `pause_vault` | -- |
| `unpause_vault` | -- |
| `slash_keeper` | `amount`, `reason_code` |
| `propose_authority` | -- |
| `accept_authority` | -- |

**Depositor** -- signed by the position owner:

| Instruction | Arguments |
|---|---|
| `deposit` | `stope_id`, `amount` |
| `request_redemption` | `stope_id`, `ticket_index`, `shares` |
| `claim_redemption` | `stope_id`, `ticket_index` |

**Keeper** -- bonded operators:

| Instruction | Arguments |
|---|---|
| `bond_keeper` | `amount` |
| `unbond_keeper` | `amount` |
| `accrue_yield` | `seam_id`, `stope_id`, `amount` |

Authority handover is two-step -- `propose_authority` then `accept_authority` -- so a typo
in a transfer cannot orphan the protocol. The vault initialises paused (`lib.rs:78`), so a
deployed-but-unreviewed program cannot take deposits. Pausing does not block claiming a
ticket whose delay has already elapsed: that is a settled debt, and an admin able to
withhold it would be an admin able to freeze user funds.

### 5.3 Events

Sixteen events, one per state transition that an indexer needs to follow:
`VaultInitialized`, `BondVaultInitialized`, `AditRegistered`, `StopeOpened`,
`SeamRegistered`, `SeamRebalanced`, `Deposit`, `YieldAccrued`, `RedemptionRequested`,
`RedemptionClaimed`, `KeeperBonded`, `KeeperUnbonded`, `KeeperSlashed`,
`VaultPauseChanged`, `AuthorityTransferProposed`, `AuthorityTransferAccepted`.

Every instruction that changes state emits one. This is what lets the indexer reconstruct
protocol state from logs rather than by polling every account.

---

## 6. Deployment topology

| Component | Platform | State |
|---|---|---|
| `apps/web` | Vercel, project `lodz-web` | Deployed |
| `apps/service` | Railway, project `lodz-api` | Deployed |
| Anchor program | Solana mainnet | **Not deployed** |
| `@lodz/cli`, `@lodz/sdk` | npm | Packaged |
| Docs | `LodzMoney/lodz`, rendered at `/shaft` | Mirrored |

### 6.1 The program is built, not deployed

`packages/anchor-program/target/deploy/lodz_vault.so` exists and a program keypair has been
generated. Neither has been sent to any cluster -- not mainnet, not devnet, not testnet.
The id in `lib.rs:70` is what the program will claim when deployed; it is currently a local
declaration and nothing more.

This is deliberate and is described in `security.md` section 6: no transaction is submitted
to any cluster until the operator supplies a keypair path, the resolved public key, the
named cluster and a balance confirmation. `anchor build` and localnet tests are the
permitted operations.

The consequence flows through the whole system. The API reports `vault_status:
pre_deployment`, `btc_in_seams: 0.0` and `basis: target_allocation`, because zero BTC is
the true amount currently routed. Every projection is a model of what the current
catalogue would pay, not a report of realised performance, and the API says so in each
response rather than in a footnote.

### 6.2 Deployment mechanism

Both deployed components ship by `git push` to their connected repository. The platform
CLIs are used for environment variables and inspection only. `railway up` hangs
indefinitely in `INITIALIZING`; `vercel deploy` and `vercel --prod` upload stale prebuilt
output. Both are prohibited, from measurement rather than preference.

The service declares its start command in one file, `railway.json`, and there is
deliberately no `Procfile` beside it. `railway.json` takes precedence over one, so a
second file can only ever drift out of sync silently -- which is exactly how a start
command passing a literal, unexpanded `$PORT` survived unnoticed. The command is wrapped in
`sh -c` so the shell expands the variable that the platform's direct exec would not.

---

## 7. Environment variable boundary

Any variable prefixed `NEXT_PUBLIC_` is inlined into the browser bundle as plain text at
build time. The prefix is therefore a publication decision, not a naming convention.

### 7.1 Decision matrix

| Question | Answer | Where it goes |
|---|---|---|
| Does exposure cost anything? | No | `NEXT_PUBLIC_*` |
| Does it authenticate, meter or bill? | Yes | Server only |
| Does the browser need it before JavaScript can run? | Yes, and it is not a credential | `NEXT_PUBLIC_*` |
| Is it needed in the browser but *is* a credential? | -- | Neither. Proxy it. |

The last row is the one that matters. A credential the browser appears to need is a
routing problem, not a labelling problem. LODZ resolves it with server-side route handlers
(`/api/rpc`, `/api/catalog`, `/api/metrics`) that hold the credential and return only
results.

### 7.2 Applied

Browser-visible, all non-secret by construction: a project address, four feature flags, a
program id, the public API base URL, a public Solana RPC endpoint, the site URL, a social
handle and a public repository path.

Server-only: the keyed Solana provider URLs, the database URL, the Redis URL, the CORS
origin list, the display-rule thresholds and the seam source configuration.

The wallet adapter is the usual place this rule breaks, because a wallet connection
genuinely needs an RPC endpoint in the browser. It is given the public mainnet endpoint,
which carries no credential and costs nothing to expose. Calls that need the keyed provider
go through the proxy.

Verified against the build output: zero credential matches across 1,302 files in `.next`,
with a control query returning 172 files to prove the scan reads bundle contents. Method
and full commands in `security.md` sections 1.3 and 10.

---

## 8. Package layout

`packages/` is one repository, `Cryptottat/lodz-core`, holding seven workspaces:

| Directory | npm name | Role |
|---|---|---|
| `anchor-program/` | `lodz-anchor-program` | Rust program, IDL, localnet tests |
| `assay-engine/` | `@lodz/assay-engine` | Three-way yield decomposition |
| `seam-router/` | `@lodz/seam-router` | Allocation policy across seams |
| `orecart-queue/` | `@lodz/orecart-queue` | Redemption wait computation |
| `headlamp-risk/` | `@lodz/headlamp-risk` | Risk layering and exposure |
| `sdk-ts/` | `@lodz/sdk` | Client library |
| `cli/` | `@lodz/cli` | Command line interface |

The four domain packages mirror rules the service also implements. That duplication is
intentional and bounded: the service is the runtime source of truth for anything served
over HTTP, while the packages let an integrator compute the same decomposition locally
without depending on a running API. Where they disagree, the service is correct and the
package is a defect.

---

## 9. Known structural limitations

**The indexer is single-instance.** The cache and the rate limiter are both in-process.
Neither survives a restart, and neither coordinates across replicas. Horizontal scaling
requires moving both to shared storage first.

**No database is attached.** `DATABASE_URL` and `REDIS_URL` are unset and the service runs
without them. There is therefore no rate history of our own -- every historical figure
comes from a third-party series at request time, and an upstream that loses history loses
it for us too.

**The public mirror is a staged copy, not a live one.** Documents and selected sources are
pushed to `LodzMoney/lodz` by an explicit script. A change landing in a private repository
is not visible publicly until that script runs.

**The program is unaudited and undeployed.** No third-party review has been performed, and
nothing in section 5 has been exercised on a live cluster.

**Cross-source verification detects disagreement, not error.** If two sources are wrong in
the same direction, the check passes. It establishes freshness, not truth.
