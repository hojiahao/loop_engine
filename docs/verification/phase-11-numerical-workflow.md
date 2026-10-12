# Phase 11 unit 2: Candidate-to-numerical research workflow

Status: `in_progress`. Code, fixtures and documentation are being integrated.
This record is not a claim that local acceptance, remote CI, publication or
production deployment has completed. Phase 11 remains open.

## Requirement and design

One explicit, human-owned step advances one current Discovery, evaluation or
portfolio child. The immutable v2 plan binds a separate Research service,
actual IS context, fixed direction/seed and all nine policies. The candidate
comes from the verified committed model receipt; narrative hypotheses are
administrator declarations. Existing v1 plans and receipts retain their bytes.

Reuse the real numerical parsers, installed workers, data broker, global trial
population, registered evaluation/backtest results and PostgreSQL transaction
boundaries. Each child is reserved before creation. Migration 0016 binds every
parent revision to immutable source/child evidence. Missing or inaccessible
population members deny the entire trial ledger; infrastructure failure is not
a rejected factor. Registered portfolio evidence does not constitute admission.

The implementation and rollback contract are in
[ADR 0053](../adr/0053-numerical-run-workflow.md). Administrator prerequisites,
closed recipe fields, protobuf preparation, permissions and executable operator
commands are documented in the
[research run guide](../development/research-runs-cli.md#numerical-workflow-opt-in).

## Required acceptance

| Boundary | Required evidence |
| --- | --- |
| Protocol | Rust, TypeScript and Python agree on v1/v2 shape, opt-in version, independent actors, numerical capabilities and safe projections; old requests remain compatible |
| Derivation | Actual registry identity, canonical AST/FactorSpec, fixed seed, explicit policy roles, IS/PIT boundary, exact dataset and registered execution tape |
| Publication | Full digest/size references, immutable no-overwrite writes, fsync/readback, retained source/output guards, missing replay objects rejected without recreation |
| Numerical workflow | Installed CLI and loopd, PostgreSQL, local real Provider transport, installed Python evaluator and portfolio producer complete a synthetic round |
| Research outcomes | Actual registered duplicate/failed memory and coverage outcomes; no fabricated numerical trial for a pre-screened candidate |
| Authorization | Human/Discovery/Research separation, run/subject spoof rejection, no protected-data access, no unreserved child or static-pin bypass |
| Recovery | Historical receipt replay launches no worker; completed-child recovery verifies actual evidence; unavailable/corrupt evidence denies advancement without duplicate dispatch or new reservations |
| Accounting | Preserved token ceilings, cross-stage budgets, parent deadline covering preparation/execution/replay, subsequent explicit step recording durable expiry, complete global trial population and current reader authorization for historical metadata |
| Read consistency | Each multi-query state/ancestry or trial-ledger read uses a repeatable-read, read-only transaction; an independent second connection committing concurrently cannot change that transaction's snapshot |
| Transactions | Independent 2/4/8 writers, source/parent revision fencing, interruption before/after commit, unchanged immutable history |
| Regression | Numerical goldens, installed standalone evaluation/portfolio workflows, Clippy, formatting and the full workspace/container/remote gates |

## Reproduction

Run from a bootstrapped repository with the pinned Python environment installed.
Use only the disposable local PostgreSQL fixture. Never point these commands at
the production database. The installed-process tests need fresh loopd/loopctl
binaries and a freshly compiled Provider host.

```bash
export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
./scripts/pnpm.sh --filter @loop-engine/providerd build
./scripts/cargo.sh build --locked --offline --workspace --all-features --bins
./scripts/cargo.sh test --locked --offline -p loop-protocol --test run_boundary
./scripts/pnpm.sh --filter @loop-engine/protocol exec vitest run test/run-boundary.test.ts
./scripts/uv-protocol.sh run --locked --offline --no-sync pytest tests/test_run_boundary.py

bash scripts/postgres-test.sh start
./scripts/cargo.sh test --locked --offline -p loopd --lib -- \
  manifests::numerical::tests \
  manifests::tests::runtime::evaluation::portfolio::numerical \
  store::runs::tests runtime::discovery::tests::workflow \
  --test-threads=1
./scripts/cargo.sh test --locked --offline -p loopctl
bash scripts/postgres-test.sh usage
bash scripts/postgres-test.sh stop

just check
just test
just build
just doctor
```

Stop the disposable PostgreSQL fixture even when a focused command fails. Keep
temporary directories inside this project's ignored `var/tmp`, or the dedicated
task-owned temporary directory chosen by the managed test runner. On a small
host, use one compiler job and one test thread; the explicit process tests still
start their required independent writers. Use the same Cargo profile throughout
the gate to avoid multiple large build caches. Remove only this project's test
temporary directories after their owners have exited, preserving immutable
research and audit data and all unrelated project files.

## Observed evidence

The following state is intentionally conservative and must be replaced with
actual command results, commit IDs and CI links during delivery:

- Manifest code and fixtures have been formatted with the pinned Rust formatter;
  `git diff --check` passed for that work at integration time.
- The first Clippy pass completed successfully. Changes made after that pass
  require another check; the final revision's Clippy result is **pending**.
- The administrator guide's generated Python constructors for the Research
  actor, `ResearchJobBudget`, exact USD and `NumericalWorkflow` were executed
  against the installed bindings and serialized successfully. This validates
  those message constructors only, not a completed or authorized run.
- Fourteen lightweight manifest cases cover registry/seed identity, dataset and
  recipe mismatch, explicit policy mapping, tape binding, PIT/holdout denial,
  source guards, AST mismatch and immutable publication/replay failures. All
  fourteen passed in the first targeted Rust execution.
- The new `store::runs::tests::numerical::snapshot_consistency` test uses two
  real PostgreSQL connections to check snapshot stability across a concurrent
  commit. Its execution result is **pending**; it does not replace the separate
  independent-process concurrency and crash/restart gates.
- A real PostgreSQL/Python fixture covers derivation from a registered successful
  evaluation, repeated exact binding, dynamic-pin denial for ordinary lookup,
  result/source corruption and missing-object replay. This fixture passed in
  the first targeted Rust execution.
- That execution exposed a stack overflow in installed numerical advancement
  and legacy receipt replay. Enlarging only the diagnostic test stack made the
  legacy case pass; this is not acceptance. Disassembly of the unoptimized
  development service identified a 0x181d38-byte advancement poll frame plus its
  0x44e68-byte deadline wrapper. The internal `RunSnapshot` also grew to 17,200
  bytes. The fix heap-owns transition records/readers and splits per-stage
  execution/preparation to bound each call frame, preserving protocol bytes.
  Default-stack regression and installed execution must pass before merge and
  completion; the draft PR retains this unresolved acceptance record.
- Targeted TypeScript and Python run-boundary suites each pass 58 cases. The
  handwritten Python/Rust/Shell naming check passes 4,918 declarations at this
  integration checkpoint; subsequent tests must also pass the final gate.
- Fresh `loopd` and `loopctl` binaries build with all features, and the actual
  Provider host compiles. Rust protocol, installed CLI workflow,
  state/concurrency/recovery and full workspace/remote checks remain **pending**.
- No live model supplier, paid market-data download, production migration or
  formal factor admission is established by synthetic fixtures.

The delivery owner must record the tested Git revision, command outputs,
failures and corrections, Chinese task commit, remote push and successful CI
before marking this delivery complete. Do not reuse Phase 11.1's results as
evidence for the new numerical path.

## Disable and recover

Disable new numerical plan registration, or remove the optional `runs` section
to disable all new run creation/advancement while retaining owner observation.
Retain a binary that understands migrations 0015/0016, immutable plans, source
artifacts, child jobs, transitions, reservations and receipts. Do not delete
tables, down-migrate, rewrite v1 history, refund ambiguous spend or regenerate a
missing committed artifact as a substitute for recovery evidence. Deployment
and production-data eligibility remain separate documented gates.
