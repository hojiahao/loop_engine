# Phase 5 US Data Verification

Status: in progress. The XNYS session-date diagnostic and local security/
observation query are published. SEC/Alpaca development ingestion is implemented
and undergoing the full unit 2 gate below. Licensed adapters, immutable Parquet
publication, historical universe coverage and production calendar manifests
remain open.

## Session-Date Checkpoint

This independently revertible step adds `--calendar XNYS` to the existing local
NAV diagnostic. It uses the pinned `exchange-calendars==4.13.2` distribution,
installed in the root Python 3.14.4 workspace. Existing locked package versions
are unchanged. Required new transitive dependencies are retained in `uv.lock`,
and installed upstream notices are preserved.

The date-only gate checks the complete NYSE session sequence between the input
endpoints, including shared missing-session rejection. Generation is bounded to
the 2005-2026 initial research window. It uses no network or current-date defaults.
Date digests and library versions are diagnostic evidence, not full execution
calendar provenance or market-data/PIT attestation.

Local evidence on 2026-09-10:

- All 28 new calendar tests pass, including independent published 2026 holiday
  dates, early-close session dates, malformed/shifted/gapped sequences, bounded
  generation, deterministic digests and actual CLI subprocesses.
- The full 2005-01-01 through 2026-08-31 date-generation path is exercised,
  including first and last sessions. This does not download data for those dates.
- Ordinary research environment: 123 passed in 20.44s.
- Isolated declared-dependency environment: 123 passed in 23.20s.
- Full `just check` passes, including unchanged protocol compatibility,
  Rust formatting/Clippy, TypeScript checks and Python formatting/types.

An earlier full-suite run had one calendar subprocess exceed its 10-second test
timeout while the separate calendar suite passed. Calendar loading now occurs
only when requested, and the two calendar subprocess tests have a bounded
30-second cold-start allowance. Sequential ordinary and isolated full-suite
runs pass; this test timeout is not a production latency SLA.

Remote acceptance is pending at this checkpoint's commit time. No production
database, real market-data source, factor admission, holdout or backtest is
accessed. The optional gate can be reverted with its package/lockfile change,
leaving the prior NAV diagnostic and all history intact. Scope, sources and
rollback details are in `docs/development/trading-calendar.md`.

## Unit 1: Security History And Point-In-Time Queries

Requirement: resolve the stable security and available information at explicit
historical cutoffs without current-ticker backfill, restatement leakage or an
issuer/share-class identity substitution. ADR 0021 and
`docs/development/point-in-time-data.md` define the scope and executable workflow.

Implementation adds immutable, strict Python records for listing histories,
raw interval OHLCV, filing-level fundamentals and source evidence. The query
keeps business/public/ingestion cutoffs separate, rejects ambiguous versions,
preserves exact decimals and uses a single source dataset per revised observation.
The installed read-only `data-query` command consumes an actual bounded JSON
capture and emits byte/result digests, visible records and explicit unattested
development-quality labels. It adds no service, database table or dependency.

Local evidence on 2026-09-13:

- New targeted suite: 94 passed in 5.51 seconds. This includes actual CLI
  subprocesses, ticker reuse, share classes, delisting/expiry without resurrection,
  source/currency conflicts, publication/ingestion delays, exact-decimal filing
  revisions, malformed/special/changing files and bounded Hypothesis properties.
- Full `just check` passed: compatibility/generated/wire boundaries, Rust
  formatting and Clippy with `-D warnings`, TypeScript formatting/lint/types,
  Python Ruff/strict typing, and the single Python 3.14.4 workspace check.
- The documented root command was executed against the committed synthetic
  fixture. It selected `synthetic:new-a` and the original revenue fact, not its
  later restatement. Input digest:
  `sha256:e7a8342ee1a58282c3e7166a3442bdd59878bc060d0bb8c8aadbe01618fdbb23`.
  Result digest:
  `sha256:882d3172345ba63e3b2aa5a21f30dd658c1a610bee9da144cee6930b4e2fbaf2`.

Full local `just test`, `just build` and `just doctor` also pass:

- Rust: 395 passed, plus four subprocess helpers exercised by their parent
  process tests. The actual 2/4/8-writer and kill/restart matrix passed in 199.02
  seconds; the runtime/manifest library suite passed in 271.34 seconds.
- TypeScript: 116 passed (115 protocol and one existing provider-host smoke test).
  The Web package still has no functional UI test suite; this is not Phase 12
  acceptance.
- Python research: 341 passed in 85.27 seconds, including the 94 new cases.
- Python protocol: 301 passed in 1.39 seconds.
- Legacy: 216 passed, 1 skipped and 11 existing numerical warnings in 14.83
  seconds. No legacy performance conclusion is restored.
- Rust/TypeScript artifacts and both Python wheels/sdists build. Doctor verifies
  the single root CPython 3.14.4 environment and existing component health/type
  checks; it does not declare production-data readiness.

Publication and remote CI acceptance are pending at this task's commit time.
The sandbox-only test invocation could not access the Docker socket; the
authorized host run used and automatically removed the managed disposable
PostgreSQL fixture. Production was not accessed. Temporary gate logs and pytest
files are confined to this task's `/tmp/loop-engine-phase5-unit1.*` directory;
the summarized evidence above is retained when those temporary files are removed.

These tests establish local query behavior, not real-vendor coverage, licensed
PIT quality, borrow availability, a portfolio backtest or full Phase 5 completion.
Disable/remove the local command to roll back; preserve captures, digest-bearing
reports and existing immutable research/audit history. No migration is required.

Unit 1 publication is complete: `523a736` is pushed and CI run `34749353894`
passes all seven jobs, including isolated research dependencies, unified
workspace commands and the clean DaoCloud container. The 13 MiB temporary gate
directory and approximately 20 MiB of project test caches/build outputs were
removed after recording the evidence. No other project's temporary files were
included in that cleanup.

## Unit 2: SEC And Alpaca Development Ingestion

Requirement: install the requested development data libraries and deliver bounded
downloads, explicit feed/identity semantics, permission failures, immutable raw
cache and executable offline replay. ADR 0022 and
`docs/development/development-data.md` record the design, limitations and commands.

The existing Python package now owns strict TOML requests, bounded sequential
HTTPS, exact-number vendor codecs, private content-addressed cache publication
and a completion receipt. `data-fetch` uses only pinned SEC/Alpaca GET routes;
`data-replay` verifies all bytes/URLs/parameters and recomputes normalization
without network or credentials. No database migration, service, model tool,
holdout capability or production database access is added.

`alpaca-py==0.44.0` and `httpx==0.28.1` are installed in the single root CPython
3.14.4 environment. The lockfile adds their nine required distributions without
upgrading existing versions. The actual SDK's public request fields are used
for Alpaca bars; its network client is not used to bypass raw-byte/budget checks.
Installed upstream notices remain intact.

Local evidence on 2026-09-13, before the full workspace gate:

- Targeted ingestion/HTTP/codec/process suite: 134 passed in 13.31 seconds.
  Covers exact decimals, filing vintages/units/periods, all permutations of
  same-date conflicts, explicit raw/feed/asof parameters, pagination, malformed
  inputs, 401/403/429, shared budgets, stalled streams, clock regression,
  cancellation, secret echoes including JSON escapes, cache corruption and
  failed publication. Real installed CLI subprocesses replay actual cache files.
- Cache publication passes 2/4/8 independent OS writer tests. Hard kills before
  and after the atomic no-replace link preserve restart behavior. Orphan unique
  temporary links from hard kills are confined to disposed test directories and
  never treated as valid receipt/object names.
- Existing PIT query/record suite: 94 passed in 2.91 seconds. Existing within-day
  observations retain behavior; new daily intervals additionally support the
  next New York midnight exclusive end, including 23/25-hour DST cases.
- `just check` passed all generated/schema/wire, Rust formatting/Clippy,
  TypeScript formatting/lint/types and Python formatting/strict typing gates.
- The only new-suite warning is the upstream Alpaca import of
  `websockets.legacy`, deprecated by its dependency. No warning is suppressed;
  this bounded GET adapter does not use websocket transport.

The installed CLI also completed a real public SEC acquisition through the
same bounded transport. It made two successful requests from
`2026-09-13T14:34:16.451053Z` to `2026-09-13T14:34:21.779714Z` (5.329 seconds),
received exactly 3,953,190 bytes (3.77 MiB) and normalized four facts, with no
missing selected concepts. This is one small API observation, not a performance
SLA, market-price cache or a complete historical PIT/universe dataset.

- Receipt: `sha256:8ce9fbec3082bf43566d21c48a7e6c609386bd995d425fc97963df9df73d6b42`
- Company Facts: `sha256:73a86c6aedc31f77cac2ea4df5f80f0b3bd7e6eb58bb4e01444fbedf3afb9c43`
  (3,789,099 bytes).
- Submissions: `sha256:cb90ffafc5b6f997b60aa109e07008223ad35abe896ec7918fd43652b4057329`
  (164,091 bytes).
- Normalized batch: `sha256:5f6006068a1d15553a6873e48e26442f1f8833e5e96dff6bc02bebfc455130f3`
  (2,946 bytes).

Those five cache objects (sources, normalized records, configuration and receipt)
were moved from the task's temporary directory to private ignored runtime storage
at `var/data/development/sec-20260913`, then replayed successfully. They are not
committed or copied into synthetic fixtures. Alpaca has offline contract/CLI
acceptance; live verification is not claimed without actual account credentials.
No paid data or LLM request is made.

The first full workspace run exposed a pre-existing test-client deadline mismatch:
`changed_output_cannot_replay_a_successful_receipt` timed out during its initial
legitimate computation, before the corruption step. Its shared metadata client
allowed 30 seconds while the server permits a 90-second numerical RPC envelope.
The numerical fixture now requests a 95-second client timeout; ordinary metadata
fixtures retain 30 seconds. Worker, server, lease and provenance limits are
unchanged. The failed run is not acceptance evidence; its disposable PostgreSQL
was automatically removed. The final full run below must pass before publication.

The isolated declared-dependency environment passed all 475 research tests in
81.54 seconds with the same single upstream websocket deprecation warning. It
installed 49 declared/transitive packages in a disposable uv test environment;
the normal project environment remains the single root `.venv`.

The corrected full host `just test` passed:

- Rust: 395 passed plus four subprocess helpers exercised by parent tests.
  The runtime/manifest library's 84 cases passed in 256.53 seconds, including
  the previously interrupted corruption check. The 2/4/8-process/fencing matrix
  passed in 190.43 seconds.
- TypeScript: 116 passed (115 protocol, one existing Provider Host smoke case).
  This does not claim implementation of Provider transports or the Web UI.
- Python research: 475 passed, one upstream warning, in 72.80 seconds.
- Python protocol: 301 passed in 1.45 seconds.
- Legacy: 216 passed, one skipped, 11 existing numerical warnings, in 14.20 seconds.
- Disposable PostgreSQL used 752,676 KiB and was automatically removed.

A final HTTP error-classification correction preserves `upstream_unavailable`
for 503 responses with unsupported Retry-After values, instead of mislabeling
them as 429 rate limits. It adds three focused cases after the full run above.
The final ingestion suite passes 137 cases in 13.54 seconds; Ruff/format/strict
types pass. The final HTTP suite also passes 39 cases in an isolated declared-
dependency environment in 3.65 seconds. Remote CI must verify the complete final
tree (478 research cases).

`just build` and `just doctor` pass on the final source: Rust/TypeScript artifacts
and both Python wheels/sdists build, and doctor confirms the root CPython 3.14.4
environment and existing CLI/research health/Provider Host type checks. These
health checks do not declare later-phase UI, LLM or production-data readiness.

Commit/push/remote CI remain the publication gate. Phase 5 and unit 2 are not yet
marked complete. Rollback disables the new CLI writers and preserves source artifacts,
completed receipts and research/audit history; no destructive down-migration is
required. Normal test artifacts are isolated in
`/tmp/loop-engine-phase5-unit2.BhkAnj` for cleanup after evidence is summarized.
