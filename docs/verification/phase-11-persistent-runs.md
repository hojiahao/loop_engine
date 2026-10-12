# Phase 11 unit 1: Persistent runs and cross-job budgets

Status: implementation and local acceptance passed. Publication requires one
task commit, push and its exact-commit remote CI; use the attached GitHub checks
as the publication receipt. Design: ADR 0051. Operator workflow:
`docs/development/research-runs-cli.md`.

## Delivered scope and boundaries

An explicitly registered human selects a frozen run plan. The run reserves each
whole Discovery job before insertion and advances through bounded rounds using
the existing Agent Harness. Jobs, command receipts and audit remain in the same
PostgreSQL transaction. Exact USD and token reservations survive restarts and
uncertain model invocations. Both ordinary job insertion and the database writer
fence reject unbudgeted children for managed run IDs.

Human owner and Discovery executor use distinct identities and certificates.
Start and step resolve the trusted catalog; disabled execution retains authorized
status with `plan_verified=false`. No Provider wire protocol, numerical engine,
holdout authority, admitted factor or unattended background loop is added here.
The only new persistent aggregate is `research_runs` in migration 0015.

## Required acceptance

- Installed `loopctl` and `loopd`, actual PostgreSQL and compiled Provider with
  synthetic HTTP supplier: two rounds, restart between rounds, exact retained
  reservations, original-key replay and deterministic completion.
- Insufficient cumulative allowance terminates without replacing a child;
  failed output remains infrastructure failure and preserves reservations.
- Disabled executors permit only authorized observation. Changed plans, spoofed
  roles/owners, foreign run IDs, stale revisions and both legacy insertion paths
  cannot bypass run authority or budgets.
- Two, four and eight independent writer processes race start, advance and replay;
  kill/restart before and after reservation/advancement commits preserves atomic
  parent, child, receipt and audit state.
- Corrupt projection/history, clock regression, deadline, unsupported protocol
  and missing evidence fail closed. Existing frozen Discovery descriptors remain
  explicitly compatible without rewriting immutable plan bytes.
- Rust formatting/Clippy, three-language contracts, existing CLI regressions and
  complete remote workspace/container gates pass before closing this task.

## Reproduce the focused workflow

Run from the repository root after bootstrap. The PostgreSQL helper manages only
the disposable local fixture; do not substitute the production database URL.
The installed-process cases require fresh binaries and the compiled Provider.

```bash
bash scripts/postgres-test.sh start
./scripts/pnpm.sh --filter @loop-engine/providerd build
./scripts/cargo.sh build --locked --offline -p loopd -p loopctl --bins
./scripts/cargo.sh test --locked --offline -p loopd \
  --test role_submission --test durable_submission -- --test-threads=1
./scripts/cargo.sh test --locked --offline -p loopd --lib -- \
  store::runs::tests runtime::discovery::tests::runs \
  runtime::discovery::plan::compatibility runtime::discovery::tests::cli \
  --test-threads=1
./scripts/cargo.sh test --locked --offline -p loopctl
./scripts/cargo.sh test --locked --offline -p loop-protocol --test run_boundary
just check
bash scripts/postgres-test.sh stop
```

Stop the disposable fixture even when a command fails. On a small host, use one
Cargo build job, disable incremental/debug artifacts and keep test temporary
files in this project's ignored `var/tmp` directory. The independent-process
tests still exercise 2/4/8 writers; build parallelism does not change that gate.

## Observed evidence

- Protocol gates: five Rust run-boundary cases, the full 165-case TypeScript
  protocol suite and the full 362-case Python protocol suite passed, including
  wire fixtures and role/RPC reachability checks.
- CLI unit regression: 76 cases passed, covering the new Human configuration,
  private plan reference, request/output projections and the existing Agent CLI.
- Existing durable/role submission regression: 34 cases passed against the real
  PostgreSQL fixture, including migration 0015, concurrent startup, replay,
  authorization, audit rollback and receipt corruption.
- Complete `scripts/check.sh` passed: Rust fmt and workspace/all-target/all-feature
  Clippy with `-D warnings`, TypeScript formatting/lint/types, Python lint/format/
  types, generated binding checks, compatibility, role boundaries and naming.
  The naming gate checked 4,697 Python/Rust/Shell and 766 TypeScript/JavaScript
  declarations.
- New run storage/process gates: all 19 cases passed, including 2/4/8 independent
  writers for start, advancement and replay, four commit-boundary kill/restart
  cuts, cumulative budget dimensions, identity/clock/deadline denial, both old
  submission bypasses, immutable receipts and recomputed-checksum corruption.
- Existing installed Discovery CLI regression: all nine cases passed, plus the
  preceding-descriptor compatibility case. All eight new installed run scenarios
  passed, including two controlled rounds/four supplier calls, restart/replay,
  exhaustion, failed child, changed-plan denial and authority boundaries.
  The changed-plan case was rerun after correcting its test-only expectation to
  the existing dependency-unavailable exit code; it confirmed zero supplier calls.
- The focused database/service gates total 71 passing cases across the executed
  runs. One process-worker entry point is intentionally ignored by the ordinary
  test runner and explicitly exercised by the process/crash gates. Final
  `loopd`/`loopctl` binaries were rebuilt before the installed-process tests.

A passing fixture is not live supplier certification or licensed-market-data acceptance.
Production at `117.50.81.155` remains on migration 0014 and its earlier stop-only
binary; no Phase 11 migration or research plan is deployed by local tests.

## Remote regression correction

Commit `719e3f7`, run `37913044037`, passed four jobs and exposed two fixture
issues in the remaining three jobs. Workspace and container acceptance exhausted
the disposable PostgreSQL 1 GiB tmpfs: temporary directories were removed, but
their migrated schemas remained until the entire suite ended. The test-only
`TestDirectory` owner now drops exactly its own path-derived `state` schema on
normal return and assertion unwinding. Cleanup has bounded connection/SQL waits;
failure fails an otherwise successful test. Subprocesses borrow paths and cannot
drop their parent's namespace. Database/principal checks and the original tmpfs
limit remain intact. Because SQLx may queue rollback/connection return on a
current-thread runtime, cleanup also terminates residual client backends only
when they belong to this test database and principal and hold a relation or
namespace-object lock in this exact owned schema. It excludes its own connection
and leaves unrelated schemas/connections intact. Normal, absent,
initialization-failure, locked-transaction and independent-owner cleanup cases
exercise the real PostgreSQL fixture, including a neighboring locked transaction
that must remain usable.

The Rust job also observed `Ambiguous` instead of `Completed` in the existing
concurrent Discovery success case. Its revision-conflict assertion passed, and
the other two jobs passed that same case. The approximately six-second duration
is consistent with the five-second fixture call deadline plus bounded lookups;
the original assertion did not retain enough outcome detail to prove the exact
dependency failure. Only that success case now uses a ten-second model/Provider
fixture budget inside the existing fifteen-second client limit. Existing
five-second timeout/cancellation/recovery cases and all production limits remain
unchanged. Strict completion, one supplier call and persisted-state assertions
remain mandatory; diagnostics record only job state, static error code, lookup
count and response presence. No uncertain call is retried as a new paid dispatch.

This correction changes test ownership and fixtures only. Reverting it does not
modify production schema or immutable research state. The corrected commit's
remote checks remain required before advancing to unit 2.

Correction validation: all 14 PostgreSQL configuration/cleanup cases passed
against the real disposable database, including both held-lock cases. All 36
selected runtime/storage cases passed, including real numerical execution,
Discovery timeout/recovery, two-round runs, receipt corruption, 2/4/8-process
contention and commit-boundary kill/restart. Two process-worker entry points are
ignored by the ordinary runner and exercised by the corresponding process tests.
The concurrent-success case then passed five additional consecutive executions.
Workspace/all-target/all-feature Clippy with `-D warnings`, Rust formatting,
the 4,711-declaration naming gate and the Provider fixture's Biome check passed.
After these runs the test-schema count was zero and PostgreSQL used 158,260 KiB
of its unchanged 1 GiB tmpfs (16%). The disposable container/network and the
empty `var/tmp/phase11-ci` directory were removed, releasing about 155 MiB of
synthetic database/WAL data. The corrected commit's remote gate remains pending.

Pre-merge documentation review compared the production bundle, migration 0015
and runtime configuration loader with the operator guides. The upgrade procedure
now distinguishes the deployed 1–14 baseline from the new 1–15 target and checks
the run-table UPDATE grant and both writer triggers. Stop-only instructions
remove `runs` together with `discovery`, matching the loader's dependency check.
These documentation corrections do not apply a production migration or enable
execution. The branch and pull-request checks remain the merge gate.

## Rollback and cleanup

Disable the optional `runs` execution configuration while retaining owner
identities and the same database. Child Discovery stop controls remain available
through their registered executor identity. Run pause/cancel controls and the
background scheduler are unit 6, not implicit capabilities of this release.

After a future migration-0015 deployment, retain all run reservations, children,
receipts, plans and audit. Use a migration-aware corrected binary; do not run a
destructive down-migration or restore an older dump over newer evidence.
Local fixtures use no paid credentials. Remove only this task's disposable
PostgreSQL namespace/container, copied test binaries and temporary directories;
preserve shared build caches, source snapshots and unrelated project files.
Local cleanup completed after acceptance: the disposable PostgreSQL container
and network were removed (about 198 MiB of test database/WAL data), together with
the task's Node compilation cache and `var/tmp/phase11` directory. No production
or other-project data was removed; these synthetic fixtures can be recreated.
