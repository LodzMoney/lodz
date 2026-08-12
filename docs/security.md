# LODZ Security Specification

Version 1. Last reviewed 2026-08-15.

This document specifies how LODZ is hardened: where secrets are allowed to exist, how
assets are identified, what the program refuses to do, and what remains exposed after all
of it. It is normative. Where a control is implemented, the file and line are named so a
reader can check the claim rather than accept it. Where something is unverified, it says
so and stays unverified.

Companion documents: `risk-spec.md` (risk measurement and disclosure), `seam-spec.md`
(yield classification and display rules), `architecture.md` (system structure).

A note on what this document is for. Most of the controls below exist because something
went wrong once, either here or somewhere public, and the entry says which. A hardening
document that reads as a list of good intentions is not auditable. One that names the
failure it is standing in front of is.

---

## 1. Secret boundary

### 1.1 The rule

Any environment variable prefixed `NEXT_PUBLIC_` is inlined into the browser bundle as
plain text at build time. It is not obscured, not minified out, and not recoverable once
shipped. A key that reaches a build output has been published, and the only remedy is
rotation.

Therefore: no credential ever carries that prefix. The keyed Solana provider
(`HELIUS_RPC_URL`, `HELIUS_WS_URL`), the database URL and the Redis URL are server-side
only, read by the API process and by Next.js route handlers running on the server.

The wallet adapter is the usual place this rule breaks, because a wallet connection
genuinely needs an RPC endpoint in the browser. LODZ resolves that by giving the browser a
public endpoint with no credential in it:

```
NEXT_PUBLIC_SOLANA_RPC=https://api.mainnet-beta.solana.com
```

That value is public infrastructure. Exposing it costs nothing because there is nothing in
it to steal.

### 1.2 The proxy

Calls that need the keyed provider go through server-side route handlers, which hold the
credential and never return it:

| Route | Purpose |
|---|---|
| `apps/web/app/api/rpc/route.ts` | Solana JSON-RPC, method-allowlisted |
| `apps/web/app/api/catalog/route.ts` | Seam catalogue, same-origin proxy to the API |
| `apps/web/app/api/metrics/route.ts` | Header metrics, same-origin proxy to the API |

The RPC route is not an open relay. It carries an explicit allowlist of fourteen read-only
methods (`ALLOWED_METHODS`, `apps/web/app/api/rpc/route.ts:23-37`) and nothing that
submits state. The reasoning is recorded in the file itself: an unrestricted proxy on a
public domain becomes somebody else's free RPC quota within a day of being found.

When the credential is absent the route answers with a `rpc_not_configured` error that
names the missing capability without printing the variable's value. The same discipline
applies to the API's own health endpoint, which reports dependencies as `configured` or
`not configured` and never echoes a URL.

### 1.3 Measurement

The boundary is checked against the build output, not assumed from the source.

```bash
grep -rlE 'api-key=|helius-rpc|laserstream' apps/web/.next/ | wc -l
```

Measured 2026-08-15 against the local build: **0** matching files out of **1,302** files in
`.next`. A control query for a string that should be present (`lodz`) returns **172**
files, which establishes that the scan reaches the bundle contents rather than silently
matching nothing. A zero without a control is not evidence.

The prefixed variables that do exist are all non-secret by construction: a project address,
four feature flags, a program id, the public API base URL, the public RPC above, a site
URL, a Twitter handle and a GitHub path.

This check belongs in the deploy gate. A single careless prefix reintroduces the exposure
at build time, and source review will not catch it if the variable is read indirectly.

---

## 2. CORS

The API runs with `allow_credentials=True` (`apps/service/src/main.py:164`). Under that
setting a wildcard origin is both rejected by browsers and a genuine hole, so the wildcard
is not merely discouraged here -- it stops the process from starting.

`apps/service/src/config/__init__.py:167-179`:

```python
for chunk in self.cors_origins_raw.split(","):
    origin = chunk.strip().rstrip("/")
    if not origin:
        continue
    if origin == "*" or origin.endswith("://*"):
        raise ValueError(
            "CORS_ORIGINS must not contain a wildcard. The API sends credentials, "
            "so every allowed origin has to be listed explicitly."
        )
```

The property that matters is where this raises. It is evaluated while the application is
being constructed, so a misconfigured deployment fails to boot rather than coming up in a
permissive state. Verified: starting the API with `CORS_ORIGINS=*` exits non-zero with that
`ValueError` and never binds its port. The bare wildcard, a scheme wildcard
(`https://*`) and a wildcard mixed into an otherwise valid list are all rejected.

Trailing slashes are stripped on the same line. An origin never carries one, and
`https://lodz.money/` compared against `https://lodz.money` is a silent match failure that
presents as an unexplained browser error rather than as a configuration mistake.

An empty result also raises. A list that parses to nothing would otherwise allow no origin
at all and look like a networking fault.

The four confirmed origins are the apex domain, the `www` host, the Vercel production
domain and the local development origin on port 3020. The `www` host is a separate origin
and is not covered by the apex entry; omitting it is the most common way this
configuration is shipped broken.

A structural note: the front end reaches the API through same-origin route handlers, so in
normal operation CORS does not arise at all. The configuration above is the second line,
for direct callers such as the CLI and the SDK.

---

## 3. Asset identification

### 3.1 Identify by mint, never by symbol

Solana carries two distinct SPL tokens with the ticker `WBTC`, both with eight decimals:

| Mint | What it is |
|---|---|
| `3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh` | Wormhole Portal wrapping of Ethereum WBTC. Accepted. |
| `5XZw2LKTyrfvfiskJ78AMpackRjPcyCif1WhUsPDuVqQ` | BitGo's canonical Solana WBTC. Denylisted. |

They have different collateral paths, different depth, and one of them is refused. Any
code that resolves an asset from the string `WBTC` picks the wrong one roughly half the
time, and the failure is silent because both resolve to something plausible.

The service therefore exposes no symbol lookup at all. `apps/service/src/config/btc_assets.py`
offers `by_mint`, `is_denied` and `assert_routable`, all keyed on the mint. Symbols exist
in the schema as display strings and are documented as such. Making the unsafe operation
absent is stronger than documenting that it is unsafe.

### 3.2 Fail closed

`assert_routable` raises rather than returning a null, and distinguishes four outcomes: on
the denylist, known but deliberately not routed, absent from the verified table, or
routable. A null return value can be ignored by a caller; an exception cannot. This is the
difference between a denylist that is enforced and one that is merely present.

### 3.3 Denylist

Three mints are refused unconditionally.

| Mint | Asset | Reason |
|---|---|---|
| `9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E` | soBTC (Sollet) | Trades 99.96 percent below bitcoin. Its bridge operator became insolvent; the token was never exploited, it was simply abandoned while 16,149 units of supply remained on chain. Its on-chain symbol is the bare string `BTC` and it has six decimals where every accepted asset has eight. |
| `21BTCo9hWHjGYYUQQLqjLgDBxjcn8vDt4Zic7TB3UbNE` | 21BTC | Economically extinct: 0.11 BTC of supply against $366 of liquidity. No usable price can be formed. |
| `5XZw2LKTyrfvfiskJ78AMpackRjPcyCif1WhUsPDuVqQ` | WBTC (BitGo canonical) | A sound asset with $46K of Solana liquidity. Any exit of meaningful size moves the price against itself. |

soBTC is the instructive entry. It shows that supply on chain is not backing, that a token
can die from its issuer failing rather than from any technical compromise, and that a
symbol match would have accepted it.

### 3.4 On-chain gate at startup

The asset table is verified against Solana before the service accepts traffic
(`apps/service/src/services/verification.py`). For every accepted mint it calls
`getTokenSupply` and checks that the supply is above zero and that the decimals match what
the code believes.

A decimals mismatch stops the boot. It means the process is pointed at a different token
than the one the table was built from, and every amount computed afterwards is wrong by a
factor of at least a hundred. Verified: injecting a mismatched decimals value into the
table causes startup to refuse with the offending mint named.

An unreachable RPC is treated differently. It is not a mismatch, so it degrades the gate
to `unreachable`, logs a warning and surfaces through `/health/detailed` rather than taking
the service down. What the gate never does is report `pass` for a check it did not perform.

Measured at 2026-08-15T08:01Z, all four accepted mints verified: cbBTC 3,396.35594396,
WBTC 2,631.90720452, zBTC 59.68411229, xBTC 352.07746268, all with eight decimals.

---

## 4. Data integrity

This section is the one written from a local failure rather than a public one.

### 4.1 The incident

An early build of the indexer shipped a file named `static_catalog.py` containing a
confident-looking set of venues and rates. The venues were real protocols, the numbers were
plausible, and none of them were measurements. It reported a Kamino cbBTC lending rate of
45 basis points where the measured rate was 0.46, roughly a hundredfold overstatement, and
it listed three emissions programmes where the measured count across 94 pools was zero.

It was served for a period, and it was served under a name that did not disclose what it
was.

That is the worst failure available to this project, because the entire product is an
argument that other people's published yield figures are not what they claim. Inventing
figures while making that argument does not merely produce a bug; it removes the standing
to make the argument at all.

Four controls came out of it.

### 4.2 A fallback must announce itself

The file is now `apps/service/src/services/sources/fallback_snapshot.py` and the source
registers under the name `fallback_snapshot`. Every rate in it is a measurement recorded in
`docs/research/btc-on-solana.md` section 7-1, unaltered. Nothing is smoothed, rounded up or
filled in.

`/health` reports the source that actually produced the data, not the one configuration
asked for. When the live pipeline degrades, the field reads `fallback_snapshot` while
`seam_source_configured` continues to read `defillama`. A health endpoint that reports the
pipeline it wishes it were using converts a visible outage into an invisible one.

### 4.3 A fallback must expire

Past `FALLBACK_MAX_AGE_HOURS` (default 72) the snapshot refuses to answer and the API
returns 503. Week-old rates presented as current are not a degraded answer, they are a
wrong one, and "unavailable" is more useful to a caller than a confident stale number.
Verified: with the limit lowered, the request fails with the snapshot's age named in the
error rather than returning data.

### 4.4 Failure is flagged, never silent

When a live read fails, the response carries `stale`, an age in seconds, a
`degraded_fallback` mode and a `degraded_reason` naming the failure. It never returns an
empty seam list, because an empty list reads as "there is no yield anywhere" rather than
"the upstream is down".

Verified end to end: with every upstream pointed at an unreachable address, the API
continued to serve 16 seams with `served_by=fallback_snapshot`, `mode=degraded_fallback`,
`live=false` and the connection error quoted in `degraded_reason`.

The background refresher distinguishes three outcomes rather than two: a success, a
failure, and a tick that completed only by falling back. Counting the third as a success
was a real defect found during this work -- the live source could be down for hours while
`/health` reported zero failures. `degraded_ticks` and `serving_degraded` are now reported
separately and `/health/detailed` reports `degraded` while it is happening.

### 4.5 Cross-source verification

Every rate that can be read from two independent sources is. DefiLlama is compared against
each venue's own API: Orca for the whirlpools, Kamino for its reserves, Save for its cbBTC
reserve. A relative gap above 20 percent suppresses the seam and flags it, because one of
the two is stale and there is no way to tell which from inside.

Two implementation notes that are load-bearing:

- Kamino reports `supplyApy` as a fraction while DefiLlama reports a percentage. Comparing
  them without conversion manufactures a 99 percent divergence on every reserve and would
  have suppressed the entire Kamino set. Unit mismatches between sources are a defect
  class, not a one-off.
- Below 0.25 percent the comparison is skipped. At Kamino's measured 0.00459 percent, a
  relative difference against 0.0046 carries no information, and a naive check would flag
  every lending reserve permanently.

### 4.6 A 200 is not a success

Every upstream response is checked for the fields it should carry before any of it is
used. The reason is specific: `api.zeusscan.io` returns HTTP 200 on every path with a
Cloudflare domain-parking page. Verified: pointing the pipeline at it produces a shape
error and a clean fallback rather than an empty or garbage catalogue.

The same principle appears on chain. Drift's BTC-PERP market holds 250 BTC of open
interest and has not processed a funding update since 2026-04-01; 200 consecutive sampled
transactions against the market account failed. Size and a live-looking endpoint are not
evidence of liveness. That market is excluded, with the measurement recorded rather than
the conclusion alone.

### 4.7 Display rules

Three rules exist to stop a technically accurate number from being a misleading one:

- Spot rates are never displayed. The Orca cbBTC/USDC pool printed 74,187 percent on one
  day of its 646-day history when its TVL momentarily collapsed. The display basis type
  has no `spot` member, so rendering one is not possible rather than merely discouraged.
- Venues below a $100K TVL floor are flagged and excluded from allocation. The Zeus Bitcoin
  Market USDC reserve advertised 104.6 percent against $10,927 of capacity.
- Points programmes are not converted to a rate. Assigning a price to unissued points
  smuggles an emissions expectation into a figure labelled as organic yield. There is no
  conversion path in the codebase to misuse.
