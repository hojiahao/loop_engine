# Source Parquet snapshots and synchronization

The installed `loop-research` CLI converts verified acquisition receipts into
immutable source Parquet, schema objects, calendar evidence and quality reports.
`data-sync` executes an explicit request plan and then runs the same snapshot
builder. These are local data-owner operations. They do not start a factor search,
register a runtime dataset, unlock a holdout or certify production data quality.

## Build and verify existing data

Use the private absolute cache directory already used by `data-fetch` or
`data-acquire`. It must be a canonical directory owned by the current user with
mode `0700`; source objects use digest-derived names. No credentials or network
are needed to convert and verify completed receipts.

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research data-snapshot \
  --store /absolute/private/source-cache \
  --receipt sha256:RECEIPT_DIGEST \
  --start 2005-01-01 --through 2026-08-31

./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research data-validate \
  --store /absolute/private/source-cache --snapshot sha256:SNAPSHOT_DIGEST
```

Repeat `--receipt` to select up to 32 unique acquisitions from that cache. Replace
the digest placeholders with actual command output. The result contains the final
snapshot reference, requested date range, partition/row counts, excluded rows and
explicit `historical_pit: not_certified` / `production_eligible: false` flags.
An empty table is valid evidence of the selected response; it is not proof that
the security had no observations or corporate actions.

`data-validate` checks the complete source graph, including original acquisition
configurations, source bytes, pagination, normalization, Parquet schema/values,
calendar, native semantics, coverage and null counts. It writes nothing. A current
implementation that produces different normalization or Parquet bytes rejects
the old manifest; retain the original locked environment to verify that historical
version. Never rewrite an old object to make validation succeed.

## Synchronize a plan

The examples are intentionally narrow selections:

- `config/data/us-development-sync.toml`: two public SEC concepts for one filer.
- `config/data/us-licensed-sync.toml`: Sharadar SEP/SF1/ACTIONS for one security,
  requiring an actual private license declaration and named API credential.

```bash
install -d -m 700 /absolute/private/source-cache

./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research data-sync \
  config/data/us-development-sync.toml --store /absolute/private/source-cache

./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research data-sync \
  /absolute/private/licensed-sync.toml --store /absolute/private/source-cache \
  --license /absolute/private/sharadar-license.json
```

The licensed example requires replacing the declaration digest; credential values
are injected through the corresponding environment secret reference. The request
and license formats are described in `licensed-data.md`. Multiple `--license`
arguments are matched by their content digests, never by arbitrary provider paths.
Only prepaid subscription access is supported; no automatic purchase or new ISIN
allocation occurs. Public SEC requests carry the configured contact email.

Each `[[requests]]` entry has exactly the same schema and limits as the existing
SEC/Alpaca/Sharadar/WRDS/Databento adapter. Split large selections by explicit dates
or identifiers when a request cannot fit its row/page budget. Plans contain no
arbitrary SQL, URLs or executable hooks. All remaining credential and license
requirements are checked before the first download.

Run `data-preflight` with the same plan, store and license arguments to inspect
those local requirements without downloads or publication. It shares the real
sync gate, checks every remaining request before sync starts and reports all
missing references together. Exit 0 means local readiness only; exit 3 means
missing/invalid access requirements, and exit 2 means malformed or unsafe input.
See `data-credentials.md` for account setup and concrete commands. A completed
offline resume still uses its original receipts and needs no current credentials.

The entire plan reserves the sum of its request budgets before execution. Limits
are at most 32 requests, 512 HTTP/SQL attempts, 512 MiB of source-response budgets,
100,000 observation records and 1,800 seconds. Each adapter retains its smaller
per-request limits. Batching is sequential; there is no hidden concurrent download
or unbounded automatic retry. Missing records remain visible in reports.

## Resume and cancellation

The command emits one JSON `sync_progress` event after each completed request.
Keep its `progress.sha256`, then resume the same plan with:

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research data-sync \
  /absolute/private/plan.toml --store /absolute/private/source-cache \
  --resume sha256:PROGRESS_DIGEST --license /absolute/private/license.json
```

Resume compares the exact plan and replays every completed receipt before it
skips any request. A fully completed prefix can build/revalidate the snapshot
offline without credentials. Unfinished requests require current credentials and
rights. A crash after a download but before progress publication may repeat that
read; no downloaded data or old progress is overwritten. Publication followed by
lost acknowledgment is handled by verifying the immutable digest on retry.

Ctrl-C propagates cancellation and closes source transports. Failed work may leave
valid source or intermediate Parquet objects; only the final snapshot manifest is
a completed snapshot. Do not treat directory contents or the newest filename as
a successful synchronization. There is no mutable `latest` pointer.

## Files, time and quality

The source snapshot uses schema `loop.source-snapshot/v1` and access scope
`private_source_only`. It references each source receipt/normalized object,
Parquet part, schema, calendar and quality report by SHA-256 and byte size.
Parts are separated into these fixed business-date ranges:

| Partition | Inclusive dates |
| --- | --- |
| warmup | 2005-01-01 – 2006-12-31 |
| in_sample | 2007-01-01 – 2016-12-31 |
| development | 2017-01-01 – 2020-12-31 |
| confirmation | 2021-01-01 – 2024-12-31 |
| recent_holdout | 2025-01-01 – 2026-08-31 |

The requested observation window is not a claim of complete data coverage. Parts
also retain each acquisition's explicit date scope. Current ticker/asset metadata
uses its observation date and cannot be relabeled as historical listing history.
Records outside the configured snapshot/source interval are counted as excluded.

Native numeric and identifier fields remain nullable UTF-8 strings, preserving
exact decimals, leading zeroes, price bases and all source revisions. The added
envelope has an Arrow `date32` business date, integer nanosecond known/ingestion
times and an explicit availability basis. SEC filing acceptance and Databento
record clocks are retained where supplied; otherwise use first observation.
The first-observed time must never be moved backward to a historical session.

Daily-price coverage is checked against pinned XNYS session dates for the selected
identifiers and their requested range. Missing sessions have counts, endpoints
and a deterministic date-list digest. Nontrading daily records fail validation.
This does not establish historical listing eligibility, venue halts, complete SIP
coverage, fundamental filing completeness, delisting returns or borrow data.
Null counts and native semantics remain in each report. No zero fill, current-
universe backfill, latest-revision selection or raw/adjusted-price conversion occurs.

The writer uses pinned PyArrow, Parquet 2.6, fixed Zstd/row-group settings and page
checksums. Each part is limited to 10,000 rows, 200 native columns, bounded scalar
and uncompressed bytes, and 64 MiB of encoded bytes. Verification checks a bounded
footer before reading values, disables extension/remote discovery and rejects
external column files. The full snapshot limits 100,000 rows, 512 parts, 512 MiB
of acquired response data and a 180-second materialization budget. These are
current implementation bounds, not an institutional-scale performance claim.

Source stores must never be mounted into discovery, LLM Provider or ordinary
research processes. The existing runtime does not accept this source manifest
schema as an authorized dataset. Phase 6 must perform the security-history,
eligibility, PIT and decision-time join before producing a broker-delivered factor
panel. Production-quality data and real protected backtests remain separate gates.

## Recovery and rollback

Disable the new CLI writers to roll back. Preserve all completed source receipts,
progress, snapshots and research/audit history. No application database migration
is added. Changed source bytes create new identities; existing references do not
silently move. Garbage collection is outside these commands: do not delete source
objects merely because a failed attempt left no final snapshot.
