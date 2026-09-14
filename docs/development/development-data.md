# SEC and Alpaca development data

The installed research CLI downloads explicitly selected SEC fundamentals or
Alpaca raw daily bars, writes private content-addressed cache objects, and verifies
them offline. The separate `data-snapshot` and `data-sync` commands now assemble
source Parquet snapshots; see `source-snapshots.md`. These development adapters
do not resolve a complete historical universe, register results or execute a
portfolio backtest. Credential onboarding and the offline `data-preflight`
command are documented in `data-credentials.md`.

## Install and run

Bootstrap/sync uses the single root Python 3.14.4 environment. The lockfile pins
`alpaca-py==0.44.0` and `httpx==0.28.1`; no additional project virtual environment
is created. After the normal bootstrap, use the root wrapper:

```bash
./scripts/uv.sh sync --all-packages --all-groups --locked
mkdir -p var/data
mkdir -m 700 var/data/my-development-cache
./scripts/uv.sh run --package loop-research --locked --offline --no-sync \
  loop-research data-fetch config/data/sec-development.toml \
  --store /home/hojiahao/loop_engine/var/data/my-development-cache
```

The store must already exist, be owned by the current user, have mode `0700`,
and be an absolute canonical path without symlinks. Substitute your own checkout
path when deploying elsewhere. `uv --offline` prevents dependency downloads;
`data-fetch` still performs its explicitly requested HTTPS data requests.

The SEC example downloads CIK `0000320193` Company Facts/submission metadata and
selects Assets and StockholdersEquity for periods ending during 2026-01-01
through 2026-08-31. Set your real contact address
in a local configuration before using the SEC API. It sends an identifying
User-Agent and at most two ordinary requests plus bounded retries. No API key is
required. There is no bulk historical filing crawl.
The SEC source endpoints do not apply that period filter: raw responses can
contain later/earlier periods and filings. Keep this unpartitioned raw cache
restricted to the data-owner process; it must not be mounted into discovery or
provider processes, or registered as an IS-only artifact. Snapshot assembly must
separate raw lineage from validated period-filtered research rows.

For Alpaca, inject the values referenced by `LOOP_ALPACA_KEY_ID` and
`LOOP_ALPACA_SECRET_KEY` from your secret manager or private shell environment,
then run:

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync \
  loop-research data-fetch config/data/alpaca-development.toml \
  --store /home/hojiahao/loop_engine/var/data/my-development-cache
```

The shipped Alpaca example requests AAPL/MSFT IEX raw daily bars during August
2026 and separately probes the latest SIP endpoint. `paper=true` selects the
paper asset-metadata endpoint for those credentials; historical market data
still comes from `data.alpaca.markets`. These are ordinary Trading API key
credentials, not a claim to support broker-partner authentication. Only GET
asset/data routes are exposed; there is no order/account mutation.

The configuration accepts explicit dates, symbols/concepts, feed, credential
reference names and finite budgets. Unknown fields, raw credential fields,
duplicate keys, endpoint overrides, today/incomplete dates, nonfinite numbers and
special configuration files fail before data publication. Errors distinguish
missing credentials, HTTP 401, HTTP 403, rate limits, malformed source data,
exhausted budgets and cache corruption. They never turn into factor rejection or
silent feed fallback.

## Budgets and access evidence

| Bound | Default | Maximum |
| --- | ---: | ---: |
| HTTP attempts including retries | 24 | 64 |
| Alpaca bars pages | 8 | 32 |
| One response | 16 MiB | 32 MiB |
| Total received response bytes | 64 MiB | 128 MiB |
| Selected source observations | 10,000 | 10,000 |
| Operation deadline | 90 seconds | 180 seconds |
| Retries per GET | 1 | 2 |

Requests are sequential with at least 0.5 seconds between starts. SEC's published
limit applies per user across all machines; operators must coordinate other
clients as well. A server retry delay is respected within the remaining budget,
otherwise the operation stops. Redirects, arbitrary hosts, ambient proxies,
cookies and compressed responses are rejected. HTTPS certificate verification
remains enabled. HTTPX phase timeouts and the async operation deadline bound
network waiting; checks also surround decoding/publication. Synchronous bounded
JSON/filesystem work and OS scheduling do not provide a hard real-time SLA.

Alpaca pagination follows each returned cursor even when a page is smaller than
the limit. Repeated cursors, missing final pages, duplicated bars, unknown symbols,
bad identifiers and rows outside the configured range reject the whole
acquisition. Successfully downloaded source evidence can remain after failure,
but no completion receipt is published for an incomplete acquisition.

Historical `sip` acceptance is distinct from latest SIP acceptance. Reports use
`recent_sip=not_requested|response_permitted|forbidden`. A successful latest
endpoint response says only that the request was permitted; it does not certify
the returned trade's freshness, real-time redistribution rights or future
entitlement. Historical failure aborts without trying another feed.

## Data semantics

SEC facts preserve CIK, accession, concept, unit, exact decimal and instant or
duration period. Company Facts contains multiple filing vintages. This adapter
selects the latest filed vintage observed for a concept/unit/period and rejects
same-date conflicting values regardless of input ordering. Equivalent decimal
scales are numerically compared with exact Decimal values. All original
vintages and submission metadata remain in the raw cache.

`known_at=ingested_at` is the actual local response-observation time. Neither
`filed`, accounting period end nor an unverified acceptance field is substituted
for historical public dissemination. Current SEC ticker lists are not converted
into listing histories. These normalized values cannot be used as if they were
available at an earlier historical decision.

Alpaca prices remain `adjustment=raw`, USD and feed-specific. The request uses
the official SDK's public request fields, explicit UTC-encoded New York date
bounds, `1Day`, ascending sort and an explicit symbol `asof` date. Each requested
current asset needs an observed nonzero UUID and matching symbol/class. This
records current vendor identity without inventing an issuer, ordinary-common
classification or historical listing/delisting facts. Crossing a New York date
during identity/bar acquisition rejects the operation.

A daily bar's timestamp labels the New York aggregation day. Its interval ends
at the next local midnight, exclusively; DST can make this 23 or 25 hours.
Provider trade-condition rules govern each OHLCV field. Volume and VWAP may use
different eligible-trade sets, so this adapter does not manufacture dollar
turnover by multiplying them. Quotes, execution prices, corporate actions,
delisting returns, borrow and session completeness remain unverified.

All output declares `public_development`, `historical_pit=not_verified`,
`universe_coverage=not_verified` and `calendar_validation=not_performed`. Empty
concept/symbol selections are explicit. A verified master/calendar join and
later snapshot quality gates are required before these observations can become
research inputs. Ingestion is not an Agent tool and must run under a data-owned
identity; do not mount its cache into discovery or provider processes.

## Cache and offline replay

Successful original response bytes, canonical configuration, normalized batch
and final receipt each have their own SHA-256 filename and exact byte count.
The cache reuses the existing atomic no-replace publisher. Credentials, HTTP
error bodies and cookie headers are not written. Exact raw and JSON-decoded
credential echoes are rejected before publication. This is not a classifier for
arbitrarily encoded or unrelated confidential vendor data.

The command prints a small summary and `receipt.sha256`. Replay it with:

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync \
  loop-research data-replay \
  --store /home/hojiahao/loop_engine/var/data/my-development-cache \
  --receipt sha256:<64-lowercase-hex-digits>
```

Replay has no network or credential requirement. It reads bounded regular files,
verifies every size/digest, validates request URLs/parameters and pagination,
recomputes normalization from original bytes, and compares exact normalized
output. Missing/corrupt data or semantic drift fails without rewriting evidence.
Hashes prove local content consistency, not a supplier signature or entitlement.
Run under normal private-directory ownership; an owner who can replace an entire
receipt and its artifacts can create another locally consistent capture.

Receipt publication is the local commit point. Cancellation before it leaves
no success receipt. Cancellation after it may prevent acknowledgment, so inspect
the cache before retrying. Repeated identical publication reuses existing bytes.
A hard kill may orphan a unique `.loop-build-*` temporary hard link; it is never
a valid digest object or receipt. Remove only validated orphan files in a stopped,
exclusively owned store. Normal completion/failure cleans its temporary link.

On this host, the bounded live SEC check is retained in the ignored private
`var/data/development/sec-20260913` store. Its receipt is
`sha256:8ce9fbec3082bf43566d21c48a7e6c609386bd995d425fc97963df9df73d6b42`.
The source data is not in Git.

The 2026-09-14 real SEC/Alpaca acceptance is retained separately in
`var/data/development/connectivity-20260914`. It verifies six IEX daily bars for
AAPL/MSFT on 2020-12-28 through 2020-12-30 and eight SEC facts selected in 2020.
Both receipts replay offline. The combined source snapshot
`sha256:3846dc0bc1140095cd7c50026334bc752ff92a26e3eeff38d070b1574b0d8370`
passes complete graph/Parquet validation, with 14 rows and two current asset
records correctly excluded from the historical window. Each selected stock has
three expected sessions, three observations and no missing session. Latest SIP
access is `forbidden`; this does not determine separate historical SIP access.
Historical PIT, universe and delisting coverage remain uncertified. See
[the verification record](../verification/phase-05-us-data.md#live-sec-and-alpaca-acceptance-2026-09-14)
for immutable identities, exact scope, timing and offline replay commands.

## Verification and rollback

```bash
./scripts/uv-research.sh run --locked --offline --no-sync pytest \
  tests/test_fetch_http.py tests/test_fetch_codecs.py \
  tests/test_development_ingestion.py tests/test_fetch_processes.py
just check
just test
just build
just doctor
```

Tests exercise real cache files and installed CLI replay, synthetic HTTP
contracts, precision/restatement/identity boundaries, retries/deadlines,
cancellation, corruption, 2/4/8 independent OS writers and kill/restart across
publication. Synthetic fixture success is not a real-vendor live test.

Rollback disables `data-fetch`/`data-replay` and their new writers without
destroying captured source objects or immutable research/audit history. There
is no database migration or production deployment in this unit. Source changes
are included in the existing worker build capture and invalidate dependent
provenance normally. Licensed source acquisition and Parquet publication are
implemented separately; actual licensed historical coverage remains unverified.

Primary references: [SEC APIs](https://www.sec.gov/search-filings/edgar-application-programming-interfaces),
[SEC fair access](https://www.sec.gov/about/developer-resources),
[Alpaca bars](https://docs.alpaca.markets/us/reference/stockbars),
[Alpaca aggregation/feed rules](https://docs.alpaca.markets/us/docs/market-data-faq),
[asset lookup](https://docs.alpaca.markets/us/reference/get-v2-assets-symbol_or_asset_id),
[latest trades](https://docs.alpaca.markets/us/reference/stocklatesttrades-1).
