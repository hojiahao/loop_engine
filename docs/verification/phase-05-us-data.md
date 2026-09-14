# Phase 5 US Data Verification

Status: in progress. Units 1-3 (local security/PIT queries, SEC/Alpaca development
ingestion and licensed source acquisition) are published with successful remote
CI. Unit 4's Parquet/synchronization implementation is undergoing its final gates.
Actual licensed historical universe/PIT coverage and authorized production-data
admission remain distinct from successful source acquisition and file validation.

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

Unit 2 publication is complete: `cc462da` is pushed and run `34765587180` passes
all seven jobs on that exact commit, including Rust, isolated Python research,
protocol/legacy regressions, TypeScript, unified workspace commands and the
clean DaoCloud container. This closes unit 2, not Phase 5 or live Alpaca access.

After summarizing local evidence, the 19 MiB project-specific temporary directory,
307 MiB of Rust incremental intermediates and about 26 MiB of project test/type
caches and package build outputs were removed. These are logical sizes; shared
hard links affect physical reclaimed space. The 3.8 MiB real SEC cache remains
in its private ignored runtime directory. Other projects' temporary files were
not included. Disk free space is approximately 5.9 GiB after cleanup.

## Unit 3: Licensed source acquisition (2026-09-14)

ADR 0023 implements the agreed Sharadar and optional WRDS/Databento delivery
unit in the existing Python data package. The installed CLI has `data-acquire`
and offline `data-verify` workflows, strict private license declarations, named
credential references, bounded source reads, immutable raw/normalized objects
and completion receipts. Configuration and operational limits are documented
in `docs/development/licensed-data.md`; four example TOML files grant no rights
and deliberately require a real declaration digest before acquisition.

Installed and locked in the existing root Python 3.14.4 environment:
`nasdaq-data-link==1.0.4`, `databento==0.86.0`, `psycopg[binary]==3.3.5` and their
transitive dependencies. The lock resolves 74 packages; sync installed 16 new
distributions and rebuilt the research package, without changing the existing
dependency versions. No new project virtual environment or application schema
is created.

Sharadar uses fixed Tables endpoints and terminal cursor pagination. Explicit
source column names protect against reordered values; split-adjusted OHLCV and
separate raw/fully adjusted closes remain distinct. As-reported fundamental dates
are retained without inventing historical dissemination times. Textual source
identifiers preserve leading zeroes; nullable corporate-action composite keys
have deterministic IDs rather than treating a changed value as a new event.

Databento uses native reference POST forms, uncompressed JSONL and the SDK's
timestamp encoding. It retains all supplied vintages, source nanoseconds and
cancellations, explicitly disables new ISIN allocation, and distinguishes valid
empty responses from missing artifacts. Unhandled provider warnings fail closed.
WRDS uses a fixed TLS PostgreSQL endpoint, parameterized read-only CIZ/fundq
projections and bounded transactions. CIZ delisting returns are not applied twice;
currently supplied Compustat revisions are not labeled verified historical PIT.

Preliminary focused evidence:

- 100 tests passed in 8.87 seconds across the first licensed-source contracts
  and the existing HTTP boundary suite.
- 144 tests passed in 13.21 seconds after adding current-reference rights,
  JSONL secret-echo, provider-warning and rehashed-normalization checks, including
  the existing development-ingestion regression. One existing upstream Alpaca
  websocket deprecation warning remains visible.
- The final targeted licensed HTTP/codec set passed 66 tests in 5.29 seconds.
- Real PostgreSQL tests passed 7 cases in 2.52 seconds: both source profiles,
  exact projection/filtering, row overflow, write rejection, statement timeout,
  cancellation and missing-column failure. Every connection closed; the unique
  synthetic database was removed without FORCE. A prior sandbox-only connection
  attempt was denied by the local TCP restriction and is not acceptance evidence.
- Full host `just check` passed Rust fmt/Clippy, TypeScript, Python Ruff/format/
  strict types, protocol-generation and workspace checks.

The final full host `just test` passed on the complete implementation:

- Rust: 395 passed, with four subprocess helpers exercised by their parent
  process tests. Runtime/manifest tests took 260.98 seconds; the 2/4/8-process
  fencing and kill/restart matrix took 203.28 seconds.
- TypeScript: 116 passed (115 protocol and one existing Provider Host smoke case).
  This is not acceptance of the later Provider platform or Web UI.
- Python research: 551 passed in 89.22 seconds, including all seven actual
  PostgreSQL source tests. One existing upstream websocket deprecation remains.
- Python protocol: 301 passed in 1.24 seconds.
- Legacy: 216 passed, one skipped and 11 existing numerical warnings in 18.15
  seconds. Historical stale performance remains invalid.
- The disposable PostgreSQL fixture used 703,060 KiB and was automatically
  removed. No production application database was accessed.

`just build` and `just doctor` also passed. Rust/TypeScript artifacts and both
Python wheel/sdist packages build. Doctor confirms the single root CPython
3.14.4 environment and the existing component health/type checks. The four
documented acquisition configurations parse through the installed implementation;
their placeholder license digests do not authorize downloads.

Publication and remote CI remain the exit gate at this task's commit time.
Unit 3 and Phase 5 are not yet marked complete. No licensed vendor live call,
snapshot admission or backtest conclusion is claimed. All supplier contract
records used here are invented data. Rollback disables the new writers and
retains completed receipts and research/audit history. Temporary gate logs and
fixtures are confined to this task's directory and identified pytest directories;
they can be removed after preserving this evidence, without deleting real caches.

Unit 3 publication is complete: `32bafdb` is pushed and CI run `34804345277`
passes all seven jobs on that exact commit, including the isolated research
dependency environment, unified workspace and clean DaoCloud container gates.
The 17 MiB task directory, 1.5 MiB of identified pytest fixtures and approximately
30 MiB of project test/type caches and package build outputs were removed after
recording the evidence. The real SEC cache and other projects' files remain intact.

## Unit 4: Source Parquet snapshots and bounded synchronization (2026-09-14)

ADR 0024 and `docs/development/source-snapshots.md` define the receipt-to-Parquet
workflow. `data-snapshot` replays source evidence, keeps native columns/precision
and distinct business/knowledge/ingestion times, partitions fixed periods through
2026-08-31, and publishes checksummed Parquet/schema/calendar/quality objects before
the final manifest. `data-validate` replays the complete graph and compares actual
Parquet schema and values. Current asset metadata is excluded from past periods;
no historical universe, verified PIT quality or protected access is inferred.

`data-sync` executes strict source request plans using the existing five adapters,
with aggregate budget reservations, credential/license preflight, secret-echo
rejection and immutable progress after every completed request. Resume verifies
the exact plan and source prefix before skipping work. No mutable latest pointer,
service, database table or new research capability is added. PyArrow 25.0.1 was
already installed/locked; it is now an explicit research dependency. No existing
package version changed and no additional virtual environment was created.

Focused evidence:

- Initial source snapshot suite: 28 passed in 94.35 seconds, including installed
  CLI round trips and 2/4/8 independent OS processes publishing the same snapshot.
- Initial synchronization suite: 17 passed in 8.58 seconds, including completed
  offline resume, interrupted prefixes, source/plan corruption, current access
  preflight, reserved budgets, cancellation and actual TOML/CLI validation.
- Expanded combined suite: 51 passed in 102.02 seconds; includes clock regression,
  nontrading daily data, empty-market coverage and credential-echo prevention.
- Real hard kills immediately before and after final-manifest publication both
  recover to the same digest. The focused fault/deadline set passed five tests
  in 8.78 seconds. The final full suite additionally checks that aggregate source
  byte limits reject before raw replay and extends both real PostgreSQL source
  profiles through the snapshot builder and offline validator.
- Full `just check` passed; after the final source-budget correction, research
  Ruff/format and strict types pass again. Both committed sync examples parse
  through the installed implementation. Their dates reach 2026-08-31; the
  requested window alone does not claim complete underlying coverage.

The retained real SEC cache was converted and revalidated locally with no new
HTTP request. The source receipt remains
`sha256:8ce9fbec3082bf43566d21c48a7e6c609386bd995d425fc97963df9df73d6b42`.
The new source snapshot is
`sha256:95657c0782edb87d2ef3fc4b572628a546bcc5b48b7259a11f7cb67feb2bdaee`
(1,280-byte manifest), containing four facts in one partition and zero excluded
rows. Its declared observation window is 2005-01-01 through 2026-08-31, while the
actual selected source facts remain limited. It explicitly reports
`historical_pit: not_certified` and `production_eligible: false`. The complete
private source cache is approximately 3.9 MiB and remains outside Git.

The final full host `just test` passed:

- Rust: 395 passed, with four subprocess helpers exercised by their parent tests.
  The runtime/manifest library passed in 266.29 seconds; the actual 2/4/8-process
  fencing and kill/restart matrix passed in 191.78 seconds.
- TypeScript: 116 passed (115 protocol and the existing Provider Host smoke case).
- Python research: 605 passed in 133.44 seconds, including the 54 new snapshot/
  synchronization cases and both real PostgreSQL-to-Parquet source profiles.
  The single existing upstream websocket deprecation remains visible.
- Python protocol: 301 passed in 1.34 seconds.
- Legacy: 216 passed, one skipped and 12 existing numerical warnings in 14.11
  seconds. No stale historical performance conclusion is restored.
- The disposable PostgreSQL fixture used 703,532 KiB and was automatically
  removed. No production database or licensed vendor was accessed.

`just build` and `just doctor` also pass: Rust/TypeScript artifacts and both
Python wheel/sdist packages build, and doctor confirms the single root CPython
3.14.4 environment and existing component health/type checks. These checks do
not imply a completed UI, Provider platform or production research workflow.

Commit/push and remote CI remain the publication gate. Unit 4 and Phase 5 are
not yet closed. Actual licensed market-data quality and the Phase 6 security/PIT
panel join remain required before production factor research.
Rollback disables new writers and preserves immutable progress, snapshots and
research/audit history. Temporary artifacts stay under this task's identified
`/tmp/loop-engine-phase5-unit4.*` directory for removal after evidence is recorded.
