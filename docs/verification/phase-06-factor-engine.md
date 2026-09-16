# Phase 6 factor engine verification

Status: in progress. The owner defers Sharadar subscription and authorizes
continued development with synthetic/public-development evidence. Phase 5's
licensed historical coverage and production-admission exit is deferred, not
passed. Phase 6 unit 3 and the later primary-backtest/independent-validation
stages remain open. Units 1–2 are published and verified.

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

Implementation `39959d7` is pushed. All seven jobs in CI run `34924328910` pass:
Rust 7m14s, TypeScript 25s, research 1m41s, protocol 32s, legacy 29s, unified
workspace 10m43s and DaoCloud container 12m13s. This includes independent worker
namespace isolation and clean-container acceptance. Passing these cases does
not establish a full-market throughput SLA, licensed PIT coverage, a portfolio
backtest, factor admission or a completed Phase 6.

## Unit 2: Frozen cross-sectional transformations

Implementation follows ADR 0027 and `docs/development/cross-sectional-transforms.md`.
The existing panel builder produces version-2 panels with the actual frozen
policy documents and optional causal exposure CSV. The existing Rust resolver,
artifact broker, installed Python worker and fenced completion handle these
artifacts without a new RPC, service, dependency or database schema.

The numerical profile clips linear-quantile tails, fits declared equal-weight
industry/log-size/beta OLS and optionally standardizes with ddof=1. Raw coverage
and every session outcome remain in the result. Sparse/rank-deficient inputs
produce explicit missing outcomes; unknown policies, numerical failures, changed
evidence and exhausted budgets fail the operation. Version-1 raw inputs reject
nonempty transformation policies instead of silently ignoring them.

The initial targeted run passes 92 Python cases in 229.17 seconds, including
raw/source/worker regressions and 18 cross-sectional goldens/properties. It
exercises the installed worker, hand/SciPy comparisons, frozen policy mismatch,
missing exposure coverage and microsecond-late revisions. Further cases cover
exposure-only corruption/time checks, bounded design work and preprocessing
without an exposure artifact. Five additional Rust integration cases exercise
version-2 completion, restart, policy mismatch, raw-policy refusal and corrupted
exposure inputs through the actual TLS/PostgreSQL/worker path.

Local `just check` and `just doctor` pass. The complete Rust test compilation
was interrupted on the approximately 1.6 GiB host after both ordinary and
single-job builds encountered sustained paging/I/O wait (77–78% in the sampled
interval). Neither interrupted attempt is counted as a passing full-suite gate.
The local research suite records 734 passed and one failed in 365.06 seconds.
All 42 new unit-2 cases pass, including actual installed-worker OLS, clipping/
standardization without exposures, temporal parsing and numerical properties.
The existing `test_independent_snapshot_writers[8]` exceeds its 90-second process
exit deadline under host resource pressure, after emitting a snapshot report;
the 2- and 4-writer cases pass. The timeout remains unchanged and is not treated
as success. The isolated 8-writer rerun passes in 85.12 seconds, with its original
deadline and assertions. The established GitHub Actions Rust, unified-workspace
and clean-container jobs must still execute the complete behavior/build/isolation
gates on the published commit. Publication and remote acceptance are pending at
this checkpoint; this is not yet unit/phase closure.

Implementation `e881d3a` is pushed. CI run `34934903854` reaches the actual
Rust/Python integration: transformed completion, restart, exposure corruption
and policy binding pass. The raw-policy negative test expects `Unavailable`,
while the existing transport correctly maps invalid input to `InvalidArgument`.
This single wrong assertion fails the Rust/unified/container gates. The follow-up
corrects that expected code; it does not change the guard, transport mapping or
numerical implementation. Full acceptance must use the follow-up commit's CI.

Correction `a1ec5e3` is pushed. All seven jobs in CI run `34947453231` pass:
Rust 9m09s, TypeScript 25s, research 2m01s, protocol 19s, legacy 49s, unified
workspace 10m49s and DaoCloud container 13m45s. This closes unit 2, including
complete behavior/build/isolation acceptance; it does not close unit 3 or
licensed-data/portfolio gates. Task-scoped local unit-2 logs and fixtures were
removed after recording this durable summary; real market caches were retained.

## Unit 3: Evaluation trials and admission lineage

Implementation follows ADR 0028 and `docs/development/evaluation-trials.md`.
Actual numerical success atomically records a verified evaluation projection
and frozen coverage disposition with its existing job/receipt/audit. Submission
and lease acquisition consult completed evidence; trial reads return verified
numerical provenance. File-backed version-2 admission reports name the actual
numerical predecessor and share the existing admission/readmission handler.

Verification is in progress. The added cases exercise actual installed-worker
coverage, trial reads/replay, duplicate/failure filtering, corrupt projections,
unverified rejection refusal, version-2 review lineage, exact context/coverage
boundaries and independent 2/4/8 writers with kill/restart around commit.
The admission fixtures deliberately supply synthetic portfolio results; they
cannot establish a completed Phase 7 engine or a real investment conclusion.
No production database, paid dataset or LLM API is used for this acceptance.

The first complete local `evaluation` run on 2026-09-16 records 22 passed,
3 failed and one subprocess helper explicitly exercised by its parent tests
(757.29 seconds). Actual worker/replay, coverage memory, migration, corruption,
unverified rejection and kill/restart cases pass. Both admission cases exceed
the legacy 10-second cold-file budget before a decision; v2 preparation now uses
the evaluator's existing 30-second bound for the same full native environment.
The independent-writer case exceeds the unchanged per-process verification
budget while hashing builds concurrently. Its setup now verifies each process
before releasing the shared 2/4/8-writer commit barrier, within the existing
five-minute lease limit. This failed run is not counted as acceptance; the
corrected workflow and exact-commit remote gates must pass.

The corrected local workflow passes all nine parent/behavior tests in 582.88
seconds (one ignored subprocess entry point is explicitly invoked). This covers
actual evaluated values through semantic rejection/readmission/replay, fabricated
coverage denial, corrupt-record refusal, duplicate/failure filtering, baseline
migration, restart, 2/4/8 independent simultaneous commits and kills immediately
before/after commit. Each database writer verifies its own byte-backed evidence;
only this preparation is serialized. The original coverage and file checks,
per-process numerical verification bound and production lease limits remain in
force. Portfolio artifacts in admission tests remain synthetic.
The focused legacy-review regression also passes (1 test, 1.00 second): a v1
review cannot create new file-backed admission. Formatting and whitespace checks
pass. Publication and full workspace/container acceptance must use this delivery
commit's remote gates; these local results alone do not close the phase.
Final `loopd` Clippy (`--all-targets --all-features -- -D warnings`) passes
against the corrected code in 6m45s on the development host. No warnings are
suppressed, and no numerical/corruption check is relaxed to obtain acceptance.

## Rollback and retained evidence

Disable the new administrative/evaluation/admission writers if rollback is needed.
Keep all real source caches, completed construction receipts, derived panels,
jobs and audit history. Units 1 and 2 do not change the database schema; unit 3
adds migration 0010. Retain that additive schema and use a compatible reader,
without a destructive down-migration. Old receipts require their original
builder source/environment for exact replay; never rewrite an old receipt to
match a newer implementation.

The committed fixture contains only invented data. Local gate logs and pytest
outputs are task-scoped temporary evidence; after their results are summarized,
remove those identified temporary files and the managed test container. Do not
remove real development market-data caches or another project's `/tmp` entries.
