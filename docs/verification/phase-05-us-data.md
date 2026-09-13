# Phase 5 US Data Verification

Status: in progress. The initial XNYS session-date diagnostic and the local
security/observation point-in-time query are implemented. Real vendor adapters,
entitlement discovery, immutable Parquet publication, historical universe
coverage and production calendar manifests remain open.

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
