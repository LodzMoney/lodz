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
| npm packages | `lodz-cli`, `lodz-sdk`, `lodz-assay-engine`, `lodz-headlamp-risk`, `lodz-seam-router`, `lodz-orecart-queue` |

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

`apps/web` has zero dependencies on any `lodz-*` package. Verified: its `package.json`
lists no `lodz-*` entry and no `workspace:` protocol dependency.

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
          apps/web (proxy)          lodz-cli                lodz-sdk
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
