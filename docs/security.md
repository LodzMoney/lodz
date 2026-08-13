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

---

## 5. Anchor program security

Source: `packages/anchor-program/programs/lodz-vault/src/`. The program defines 53 distinct
error variants (`errors.rs`), which is the shape of a program that rejects specifically
rather than generically.

### 5.1 Ownership is checked by constraint, not by hand

Account ownership is asserted with Anchor's `has_one`, so the check is part of account
resolution and cannot be forgotten inside a handler:

```rust
has_one = owner @ LodzError::Unauthorized,        // redemption.rs:64, 292, 300
has_one = authority @ LodzError::Unauthorized,    // admin.rs:161, 224, 328, 428, 562, 612
has_one = asset_mint @ LodzError::AditMintMismatch, // deposit.rs:38, redemption.rs:92, 301, 318
```

Every admin-only instruction carries `has_one = authority`. Every redemption path carries
`has_one = owner`. Mint identity is bound the same way, so a caller cannot substitute a
different asset's accounts into a matched instruction.

### 5.2 Arithmetic cannot silently wrap

`overflow-checks = true` is set in the workspace profile
(`packages/anchor-program/Cargo.toml:10`), and value-carrying arithmetic uses checked forms
that surface as errors:

```rust
// deposit.rs:103-108 -- deposit cap
let after = ctx.accounts.adit.total_deposited
    .checked_add(amount)
    .ok_or(LodzError::MathOverflow)?;
```

`checked_add` appears throughout the deposit, keeper and redemption paths;
`checked_mul` guards the fixed-point math in `math.rs:49, 90, 107, 128`.

`saturating_sub` is used deliberately and only where clamping at zero is the correct
outcome rather than a way to avoid thinking about the error case. `keeper.rs:231` reduces a
keeper's bond, and `math.rs:101` computes a yield index delta where an index that has not
advanced must produce zero rather than a negative.

### 5.3 PDA seeds are consistent across every call site

An account derived with different seeds in different instructions is not the same account,
and the resulting bug presents as missing state rather than as an error. The seed constants
are declared once and referenced everywhere:

| Seed | Call sites |
|---|---|
| `VAULT_CONFIG_SEED` | `deposit.rs:29`, `accrual.rs:47`, `keeper.rs:34`, `keeper.rs:151`, `keeper.rs:263`, `admin.rs:75`, `admin.rs:159` |
| `ADIT_SEED` + mint | `deposit.rs:36`, `accrual.rs:80` |
| `STOPE_SEED` + id | `deposit.rs:46`, `accrual.rs:73`, `keeper.rs:287` |
| `KEEPER_SEED` + authority | `accrual.rs:56`, `keeper.rs:50`, `keeper.rs:161`, `keeper.rs:270` |
| `SEAM_SEED` + id | `accrual.rs:64`, `keeper.rs:279` |
| `ADIT_VAULT_SEED` + mint | `deposit.rs:74`, `accrual.rs:100` |

Each is composed from a constant plus a discriminating key or little-endian id. No call
site builds a seed from a literal.

### 5.4 Stack frame pressure

Solana BPF programs have a 4,096-byte stack frame limit, and an instruction with many
deserialized accounts exceeds it before the logic is written. Every large account in the
deposit and redemption contexts is heap-allocated with `Box`:

```rust
pub vault_config: Box<Account<'info, VaultConfig>>,   // deposit.rs:32
pub adit:         Box<Account<'info, Adit>>,          // deposit.rs:42
pub stope:        Box<Account<'info, Stope>>,         // deposit.rs:49
pub miner:        Box<Account<'info, Miner>>,         // deposit.rs:60
pub orecart_queue: Box<Account<'info, OrecartQueue>>, // redemption.rs:74
pub orecart:      Box<Account<'info, Orecart>>,       // redemption.rs:83
```

This is a correctness control, not a style choice. Frame exhaustion appears at runtime on
the instructions with the most accounts, which are the ones that move the most value.

### 5.5 Time validation on redemption

A queued ticket cannot be claimed before its delay elapses (`redemption.rs:365-368`):

```rust
require!(
    now >= ctx.accounts.orecart.claimable_at,
    LodzError::RedemptionNotClaimable
);
```

`claimable_at` is stamped once at request time from the base delay plus a congestion term
and is stored on the ticket (`redemption.rs:162, 241`), so it cannot be recomputed
favourably later. The predicate is duplicated as `Orecart::is_claimable_at`
(`state/orecart.rs:73-74`), which additionally requires the ticket still be in `Queued`
status, and claim ordering is guarded separately by a `TicketAlreadyClaimed` check
(`redemption.rs:360`).

### 5.6 Authority scope

The authority is a single key checked by constraint, and what it can do is bounded by
compile-time ceilings rather than by trust (`state/mod.rs:74-85`):

```rust
pub const MAX_FEE_BPS: u16 = 500;                          // 5 percent
pub const MAX_BASE_REDEMPTION_DELAY_SEC: i64 = 30 * 86_400;
pub const MAX_TOTAL_REDEMPTION_DELAY_SEC: i64 = 180 * 86_400;
pub const MAX_KEEPER_UNBOND_COOLDOWN_SEC: i64 = 30 * 86_400;
```

A compromised authority key therefore cannot set a 100 percent redemption fee or convert
the queue into an indefinite lockup. The ceilings bound the damage; they do not prevent it.

Authority handover is two-step: `propose_authority` (`admin.rs:750`) followed by
`accept_authority` (`admin.rs:776`), with the proposal held in `pending_authority`
(`state/config.rs:17-19`). A single-step transfer turns one typo into a permanently lost
protocol.

The vault starts paused (`lib.rs:78`), so an initialized-but-unreviewed deployment cannot
take deposits. `pause_vault` and `unpause_vault` (`admin.rs:572, 586`) are authority-gated.
Pausing does not block claiming a ticket whose delay has already elapsed
(`state/config.rs:57-59`): that is a settled debt, and a pause that could withhold it would
be an admin key able to freeze user funds already promised.

### 5.7 Unverified

The program has not been audited by a third party. The controls above are what the source
implements, verified by reading it; they are not an assurance that the program is free of
defects. Localnet unit tests exist; there has been no mainnet or devnet deployment.

---

## 6. On-chain deployment gate

No agent or automation submits a Solana transaction -- on any cluster, including devnet --
until the operator has supplied four things explicitly:

1. the keypair path to sign with
2. the public key that keypair resolves to
3. the target cluster, named
4. confirmation of the balance available

`solana-keygen new`, `solana airdrop` and `anchor deploy` are not run on an agent's own
initiative. What is permitted is `anchor build` and localnet unit tests, neither of which
touches a live cluster or spends anything.

The gate exists because the alternative was tried. Automated deployment steps have
previously executed against live clusters without the operator intending it, and a
transaction cannot be recalled. The specific hazard is not a large loss on devnet; it is
that an automated deploy establishes a program id and an upgrade authority that then have
to be lived with.

For the same reason, no fallback or retry hook may be attached to a transaction path.
"Automatically top up and retry on insufficient funds" converts a failed deploy that
someone would have investigated into a successful one that nobody reviewed. A transaction
that fails must surface as a failure.

---

## 7. Deploy identity isolation

Real service code lives in private repositories under a single account. Public
repositories receive only what is deliberately staged for them, and only after the account
is designated. An agent does not create, push to or delete public repositories on its own
judgement.

Several build sessions run concurrently on one machine, which makes the failure mode here
specific: any command that writes global tool state silently repoints every other session.
`railway login`, `vercel login`, `gh auth switch` and `solana config set` are prohibited
for that reason. Each project uses per-project token wrappers that route by working
directory instead, so identity is a property of where a command runs rather than of when it
last ran.

Two related prohibitions, both from measured failures rather than theory:

- `railway up` is not used. It hangs indefinitely in `INITIALIZING`. Deployment is by
  `git push` to a connected repository.
- `vercel deploy` and `vercel --prod` are not used. They upload stale prebuilt output.

Installed is not the same as working. A shell profile that rewrites `PATH` after the
wrapper directory is added, or a git credential helper holding a different account, both
defeat the isolation without producing an error -- the command simply goes out as the wrong
identity. The isolation is therefore verified by a doctor command at session start rather
than assumed from the presence of the wrappers.

---

## 8. Rate limiting

The API applies an in-memory sliding window keyed on the first hop of `X-Forwarded-For`,
per instance. Preflight `OPTIONS` and the health path are exempt: a throttled preflight
presents to a browser as a CORS misconfiguration, and a platform health probe on a fixed
interval would otherwise consume the budget that users need.

One property must be understood before this limit is tuned. The front end reaches the API
through server-side route handlers, so every visitor arrives from one egress address and
shares a single bucket. The limit is an aggregate, not a per-person allowance. Either it is
sized for total traffic or the proxy forwards the original client address. This was found
by measurement, not by review: a per-IP limit sized for individuals throttled the entire
local development environment as soon as two clients polled at once.

Multi-instance deployment requires moving the window to shared storage. Until then, the
enforced limit is per instance, and this document says so rather than implying a global
one.

---

## 9. Known limitations

The controls above reduce specific exposures. These are not among them.

**Custody risk cannot be mitigated, only disclosed.** Every asset LODZ accepts is a claim
issued by a third party against bitcoin held elsewhere. cbBTC and xBTC both have mint and
freeze authorities that are ordinary keypairs, measured on chain -- their issuers can
freeze any account holding them, including a vault's, with no technical failure occurring
and no action available to LODZ. cbBTC and the Portal WBTC have no automated reserve proof
on any chain; their backing rests on issuer disclosure. No amount of program hardening
changes any of this. It is reported on every risk response and in `risk-spec.md`, and
reporting is the entire available remedy.

**Bridge risk is inherited, not managed.** The Portal WBTC path passes through the bridge
that lost $326M to a signature verification bypass in 2022. That loss was covered by a
backer, which is a fact about that backer and not a property of the bridge.

**Upgrade authority and external prices are the same two surfaces that were combined in
the Drift compromise** of 2026-04-01, which took $295M and was not recovered. LODZ will
hold an upgrade authority and will read external prices. The ceilings in section 5.6 bound
what a compromised authority can change; they do not prevent the compromise.

**The divergence loss estimate is a floor, not a forecast.** It uses a full-range constant
product formula over a short observed window, so a concentrated position loses at least
the stated amount and possibly more, and annualising a calm week understates while a
violent one overstates. Where no estimate can be produced the seam is marked
`il_unknown` and no net figure is given, rather than substituting a zero.

**Cross-source verification catches disagreement, not shared error.** If DefiLlama and a
venue's own API are both wrong in the same direction, the check passes. It detects
staleness on one side; it does not establish truth.

**Rate limiting is per instance and in memory.** It does not survive a restart and does not
coordinate across replicas.

**The program is unaudited and undeployed.** No third-party review has been performed.

**Points programmes are not measured.** DefiLlama's reward field captures token emissions
and does not capture unissued points. The measured zero-emissions finding is a statement
about token emissions specifically. No unauthenticated public source for points accrual was
found during research, and that gap is unverified rather than assumed to be empty.

---

## 10. Verification commands

Each of these was run against the working tree on 2026-08-15 with the result stated.

```bash
# No credential reaches the browser bundle. Control query proves the scan reads the bundle.
grep -rlE 'api-key=|helius-rpc|laserstream' apps/web/.next/ | wc -l     # 0 of 1302 files
grep -rl 'lodz' apps/web/.next/ | wc -l                                 # 172 (control)

# No hardcoded key in the service source.
grep -rn "api-key=" apps/service/src/ | grep -v "os.environ\|getenv\|Settings"   # 0

# Wildcard CORS refuses to start. Exits non-zero, never binds.
CORS_ORIGINS='*' uvicorn src.main:app --port 8029                       # ValueError, exit 1

# Prohibited-language scan. The pattern is written with bracket escapes so that this
# document does not itself contain the phrases it forbids, and therefore passes the
# same check it describes. The enforcing copy lives in github/gated-push.sh.
PAT='risk[-]free|guarantee[d]|native[ ]bitcoin'
grep -rniE "$PAT" apps/service/src/ docs/security.md                    # 0

# The port variable is expanded by a shell rather than passed as a literal.
grep -rnE '\$\{?[A-Z_]+' apps/service/railway.json                      # startCommand only
```

The last one is a fix rather than a check. The start command previously passed a bare
`$PORT`, which Railway execs directly, so the process received the four literal characters
and failed to parse a port. It survived unnoticed because a second file declared a
different, correct command, and the two disagreed silently. There is now one start command
in one file.
