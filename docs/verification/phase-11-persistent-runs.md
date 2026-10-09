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
