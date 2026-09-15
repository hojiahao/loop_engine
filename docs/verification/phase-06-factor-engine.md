# Phase 6 factor engine verification

Status: in progress. The owner defers Sharadar subscription and authorizes
continued development with synthetic/public-development evidence. Phase 5's
licensed historical coverage and production-admission exit is deferred, not
passed. Phase 6 units 2–3 and the later primary-backtest/independent-validation
stages remain open.

## Unit 1: Causal source-to-worker panels

Requirement: construct the authorized worker's actual inputs from captured
source evidence and explicit security histories, rather than requiring manually
assembled panel values. Preserve the calendar grid, eligibility, point-in-time
selection, frozen identities and source/worker storage boundary.

ADR 0026 and `docs/development/causal-factor-panels.md` describe the executable
administrative workflow. `panel-build` and `panel-validate` reuse the Phase 5
private CAS, PIT records, source-receipt replay, Parquet verification and pinned
XNYS calendar. Public prices can only come from a verified development snapshot;
synthetic prices remain explicitly invented. The existing runtime formats,
Protobuf messages, PostgreSQL schemas and numerical worker remain compatible.
No new service, table, dependency or provider-specific numerical branch is added.

The Rust authorization integration now invokes the installed Python builder
with the committed source/capture/request fixture. Its returned dataset and
calendar are pinned before real mTLS, PostgreSQL lease and subprocess evaluation.
The broker receives only the derived panel and CSV. The existing moving-average
golden, completion fencing, corrupt-input/output rejection and restart tests
exercise this generated input, rather than a parallel hand-built CSV path.

Targeted local evidence on 2026-09-14:

- 67 tests pass in 56.42 seconds: 49 panel/source cases and 18 build-identity
  cases. The existing third-party `websockets.legacy` deprecation warning remains.
- Numerical/time goldens cover missing sessions, warmup exclusion, late
  corrections, exact microsecond cutoffs, ticker reuse, delisting/expiry,
  excluded instrument kinds, wrong currencies/partial intervals, exact volume,
  DST and early closes. A capture one microsecond before the final decision is
  rejected; exported knowledge timestamps round upward.
- Actual source acquisition receipts, raw wire bytes and Parquet are replayed
  before public-panel construction. Current first-observed metadata/prices
  produce zero historical eligibility/observations, not invented past knowledge.
  Manual public-price injection, missing history, corrupted source Parquet,
  broad snapshot/acquisition ranges and an acquisition beyond the capture cutoff
  fail before derived publication.
- The installed build/validate CLI, read-only worker parser, deterministic
  replay, byte/source corruption, symlinks, overlapping stores, bounded work/
  cells/deadlines and cancellation before the final receipt are exercised.
  Repeating interrupted work preserves/reuses existing immutable objects.
- `describe_source()` matches the source component of the actual numerical
  build identity. A source edit during an earlier development test run correctly
  rejected construction; the stable-source targeted rerun above passes.

Full local gates pass on 2026-09-15:

- `just check`: protocol compatibility/generated/wire boundaries, Rust formatting
  and Clippy with `-D warnings`, TypeScript formatting/lint/types, Python Ruff/
  strict typing and the single root CPython 3.14.4 environment.
- `just test`: Rust 395 passed, plus four subprocess helpers exercised by parent
  tests. The seven generated-panel authorization cases pass. The runtime/
  manifest suite takes 268.08 seconds; the independent 2/4/8-writer and
  kill/restart matrix passes in 195.98 seconds.
- TypeScript 116 passed; Python research 693 passed in 184.90 seconds; Python
  protocol 301 passed in 1.43 seconds; legacy 216 passed, 1 skipped in 15.14
  seconds. Existing third-party/legacy warnings remain; old metrics stay stale.
- `just build` and `just doctor` pass for the current Rust/TypeScript artifacts
  and Python packages. These are component/build checks, not a completed UI or
  production research workflow.
- The initial Rust integration run rejected the fixture's shared `0755` output
  directory. The fixture now creates a private `0700` directory; the production
  guard is unchanged. The complete rerun above passes. The disposable local
  PostgreSQL fixture was automatically removed; production was not accessed.

Publication and exact-commit remote CI are pending at commit time. Passing these
cases does not establish a full-market throughput SLA, licensed PIT coverage,
a portfolio backtest, factor admission or a completed Phase 6.

## Rollback and retained evidence

Disable the new administrative writers and revert the unit's code if needed.
Keep all real source caches, completed construction receipts, derived panels,
jobs and audit history. No schema migration or destructive down-migration is
introduced. Old receipts require their original builder source/environment for
exact replay; never rewrite an old receipt to match a newer implementation.

The committed fixture contains only invented data. Local gate logs and pytest
outputs are task-scoped temporary evidence; after their results are summarized,
remove those identified temporary files and the managed test container. Do not
remove real development market-data caches or another project's `/tmp` entries.
