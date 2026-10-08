# Phase 10 unit 4: Discovery lifecycle

Status: implementation and local behavioral validation passed; commit `12768ff`
was pushed. Run `36689533660` passed six jobs, including Rust and the clean
DaoCloud container. The unified job passed Rust/TypeScript and reached Python
research at 96% before its 75-minute job timeout; it did not report a test
failure. Full descendant CI remains the closing gate. Unit 5 preserves all
checks and adjusts that job budget to 120 minutes. ADR 0049 defines the design and scope;
`docs/development/discovery-lifecycle.md` documents controls and rollback.

## Acceptance cases

- Real PostgreSQL/mTLS/Provider pause and resume before initial reservation,
  around the registered tool and during the second model ordinal.
- Cancel a live supplier wait; old worker cannot commit or authorize another
  model. Read-only recovery appends late evidence without changing cancellation,
  producing a candidate or clearing reservations.
- Disabled executor and damaged plan/data still permit authenticated stop
  commands. Invalid actor/run/role, corrupt job and absent caller deadline deny.
- Expired and stale Resume requests retain CAS and receipt semantics. Replays
  are observational, not another execution grant.
- Three bounded lookup attempts; transient recovery, exhaustion and pause/restart
  retain counters. Paid dispatch is never retried for ABSENT or uncertain work.
- Bad response identity/usage records infrastructure failure and retains spend;
  invalid AST remains distinct from empirical factor rejection.
- Independent 2/4/8-process control/retry/result contention; kill/restart before
  and after all new transition commits; commit-time clock regression rollback.
- Additive migration keeps old request/response bytes and prevents old writers;
  stable completion chronology survives retry metadata updates.
- Three-language Paused vectors, reviewed previous descriptor v1/v2 execution,
  original numerical/wire contracts, formatting, Clippy and workspace gates.

The PostgreSQL migration comparison preserves unrelated clock/lease constraints
as well as original row bytes. Runtime tests use the default test-thread stack;
the tool/lookup futures and recurring authority check are locally boxed so the
combined asynchronous state machine does not require a larger process-wide stack.

## Reproduction

Build `@loop-engine/providerd`, start the disposable fixture with
`bash scripts/postgres-test.sh start`, then run:

```bash
./scripts/cargo.sh test --locked --offline -p loopd --lib -- \
  runtime::discovery store::model_step runtime::tests \
  runtime::service::discovery --test-threads=1
./scripts/check.sh
```

Use `bash scripts/postgres-test.sh usage` and `bash scripts/postgres-test.sh stop` after
testing. The process tests invoke their ignored child entry point directly;
the ordinary test run must not unconditionally run that child entry point.

## Observed checks

- TypeScript protocol: 127 passed; Python protocol: 324 passed.
- Complete workspace `scripts/check.sh`: passed, including Rust formatting and
  Clippy with warnings denied, TypeScript/Python static gates, schema generation,
  wire goldens, compatibility and RPC reachability boundaries.
- Final Rust lifecycle/Provider/process regression: **159 passed, 0 failed,
  1 ignored child entry point**, including 36 actual Discovery/Provider workflows
  and independent 2/4/8-process contention and before/after-commit crash cuts.
  Default test stack and original operation deadlines remain unchanged. The
  regression includes fixes for asynchronous stack size, migration constraint
  selection, and the three corrected timeout/plan-binding test fixtures.
- Disposable PostgreSQL after repeated local suites: 61% of its 1 GiB tmpfs;
  data files 507,760 KiB and WAL 131,072 KiB. No production database was touched.

Test suppliers and development fixtures require no paid LLM calls, production
database writes or holdout access.
Remove this task's disposable database and generated temporary directories after
verification; preserve runtime dependencies and unrelated project files.
