# Point-in-time data queries

The Python data package resolves security histories and visible observations
from a bounded local capture. This is the executable Phase 5 unit 1 development
workflow. It does not download prices, certify a historical universe, register a
research result or grant access to protected storage. Provider ingestion and
immutable Parquet snapshots are separate subsequent delivery units.

## Run the synthetic example

From the repository root, after bootstrap:

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync \
  loop-research data-query fixtures/market/pit/history.json \
  --market-at 2020-03-02T22:00:00Z \
  --known-at 2020-03-02T22:00:00Z \
  --ingested-at 2026-09-13T00:00:00Z \
  --ticker DEMO --venue XNYS
```

The report selects `synthetic:new-a`, the original filing value
`9007199254740993.01`, and the visible unadjusted bar. The later filing revision
is absent. The decimal remains a string; it must not pass through a JavaScript
number or Python float. The report includes the exact input SHA-256, a SHA-256
of compact UTF-8 `PitResult.model_dump_json(by_alias=True)`, all three query
clocks and every selected record's source declaration. Repeating a query on
identical bytes produces identical output; there is no wall-clock default.

Change both decision timestamps to `2017-12-29T23:00:00Z` and the same ticker
resolves to `synthetic:old-a`. At `2018-01-02T22:00:00Z` it resolves to nothing:
the old security has delisted and its successor has not listed. At
`2019-01-02T22:00:00Z` it resolves to the new security. These are fixture facts,
not real historical market data.

Omit `--ticker/--venue` to query the default visible universe: listed ordinary
common stock on `XNYS`, `XNAS` and `XASE`. Use `--security-id` to inspect a stable
ID, including an excluded or explicitly delisted security. The response's
`universe_eligible` flag remains false for such securities. Missing selections
produce empty arrays. Ticker queries require a venue and cannot be combined
with a security-ID query. Each historical decision needs its own universe
query; never reuse today's selected IDs for a past backtest.

## Record and clock semantics

The `loop.pit-input/v1` schema accepts only `synthetic` or `public_development`
captures. Its arrays are immutable validated records, with at most 10,000 total
records and an 8 MiB file limit. The input `captured_at` fixes the latest replayable
ingestion time. All timestamps require explicit timezones and normalize to UTC.
The query requires `market_at <= known_at <= ingested_at <= captured_at`.

| Clock | Meaning | Example |
| --- | --- | --- |
| `effective_at` / `market_at` | Business event/observation time and its query cutoff | A listing starts, a price interval ends, an accounting period completes |
| `known_at` | Public availability and the information cutoff | Filing acceptance occurs after the fiscal year ends |
| `ingested_at` | Local receipt and the replay cutoff | A historical filing was downloaded in September 2026 |

Public availability is not inferred from a fiscal period, current ticker list,
or local receipt. A `first_observed` source must use its ingestion timestamp as
`known_at`; it cannot claim that data was historically available sooner. A
publisher-timestamp declaration needs upstream evidence in the adapter/snapshot
pipeline. Announcement time may precede a security event's effectiveness.

Security states retain distinct security and issuer IDs. Share classes can share
one issuer and its entity-wide facts without sharing prices or a security ID.
For each effective event, the latest visible public revision wins; the latest
effective event then defines the security state. An exclusive effectiveness end
or a delisting is applied after selection, so the query cannot resurrect an older
active version. Duplicate/conflicting versions or simultaneous listed uses of
the same venue/ticker fail closed.

Raw bars preserve their interval start/end, New York session date, currency,
unadjusted OHLCV and visibility evidence. Prices must be positive exact decimals,
low/high must contain open/close, and volume is a nonnegative integer. An ended
bar cannot be visible before its end. This unit does not assert session-calendar
completeness, regular/extended-hours coverage, consolidation, corporate-action
adjustments, executable quotes or availability to borrow.

Fundamentals preserve issuer, concept, unit, instant/duration period, filing ID
and exact decimal. `effective_at` belongs to the completed `period_end`; it is
not the publication timestamp. The latest publicly and locally visible revision
of that exact concept/unit/period is selected. Different fiscal periods and units
remain distinct. There is no implicit aggregation, currency conversion, missing
value fill, quarterly derivation or share-class join.

Revisions of one logical observation must come from the same explicit source
dataset; bar revisions must also retain currency. A later timestamp does not
authorize mixing IEX/SIP feeds or different vendors. Cross-source reconciliation
requires an explicit future ingestion policy. Duplicate versions are rejected,
even when identical; an adapter must deduplicate capture input before publication.

## Validation and access boundaries

The loader verifies bounded regular files, rejects symlink leaves/FIFOs/devices,
detects in-read metadata changes, rejects duplicate JSON keys and validates the
entire capture before returning anything. Inconsistent timestamps, references,
intervals or revisions return an error, not a factor rejection. Errors from the
CLI do not print record bodies or private input paths.

The input digest proves which bytes were queried. The per-record `raw_sha256`
is still a source declaration in this unit, not independently verified vendor
content. Accordingly, every report says `quality_verification: not_attested` and
`calendar_validation: not_performed`. Synthetic digest placeholders are explained
beside the fixture. This command cannot mark input production-grade or write
backtest performance.

This is a local developer/ingestion diagnostic subject to ordinary filesystem
permissions. It is not exposed as a model tool or a `loopd` capability. Runtime
data boundaries remain those in ADR 0019: unprivileged discovery/research
deployments must not mount protected data. A local path or CLI flag does not
unlock that storage. Production clients continue to use the control-plane API.

## Verification and rollback

```bash
./scripts/uv-research.sh run --locked --offline --no-sync pytest \
  tests/test_pit_data.py tests/test_pit_diagnostic.py
just check
just test
just build
just doctor
```

Tests cover ticker reuse, share classes, delisting/expiry, revisions and delayed
ingestion, exact decimals, source/currency conflicts, malformed records/files,
bounded reads, real CLI behavior and future-invisible/order-invariance properties.
The current implementation adds no database migration, service or dependency.
Rollback disables/removes `data-query` and its package while preserving input
captures and report digests. It does not modify immutable research/audit history.
This new package is included by the existing worker build-identity capture;
source changes naturally change provenance instead of preserving stale metrics.
