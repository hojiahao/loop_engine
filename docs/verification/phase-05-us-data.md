# Phase 5 US Data Verification

Status: in progress. Only the initial XNYS session-date diagnostic is implemented.
Security Master, price/fundamental adapters, entitlement discovery, immutable
Parquet snapshots and production calendar manifests remain open.

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
