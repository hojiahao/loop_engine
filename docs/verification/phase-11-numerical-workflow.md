# Phase 11 unit 2: Candidate-to-numerical research workflow

Status: `in_progress`. Code, fixtures and documentation are being integrated.
This record is not a claim that local acceptance, remote CI, publication or
production deployment has completed. Phase 11 remains open.

The initial implementation is published as `2ec2965` in draft
[PR #4](https://github.com/hojiahao/loop_engine/pull/4). Review fixes stay on
the same branch; neither that publication nor a pending CI run closes the unit.

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
./scripts/cargo.sh test --locked --offline -p loop-protocol --all-features --test run_boundary
./scripts/pnpm.sh --filter @loop-engine/protocol exec vitest run test/run-boundary.test.ts
./scripts/uv-protocol.sh run --locked --offline --no-sync pytest tests/test_run_boundary.py

bash scripts/postgres-test.sh start
./scripts/cargo.sh test --locked --offline -p loopd --all-features --lib -- \
  manifests::numerical::tests \
  manifests::tests::runtime::evaluation::portfolio::numerical \
  runtime::authority::tests runtime::workflow::tests \
  runtime::discovery::tests::fixture::run_operations::numerical_cache \
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
- Full workspace Clippy, including all targets/features and `-D warnings`,
  passes after the nested model/numerical frame split, final authority checks
  and installed writer fixtures. Behavioral/default-stack acceptance remains
  separate and pending.
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
  commit. It passed at `bf02ea0`; it does not replace the separate
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
  A second installed run still overflowed with the outer advancement frame
  reduced to 0x3d888 bytes. The bounded portfolio completion call chain adds
  roughly 640 KiB of storage validation to the previous 1.46 MiB runtime chain;
  Discovery also retained an approximately 784 KiB drive frame. The follow-up
  splits shared model/numerical execution and legacy dispatch, without changing
  thread-stack settings or adding detached tasks. The `bf02ea0` targeted run
  completed without stack overflow, but its installed workflow failed at the
  first numerical child reservation. Full-round acceptance remains required.
- Targeted TypeScript and Python run-boundary suites each pass 58 cases. The
  complete Python protocol suite passes 384 cases after synchronizing the
  shared fixture's current descriptor hash. Ruff and mypy also pass.
  The handwritten Python/Rust/Shell naming check passes 4,944 declarations at this
  integration checkpoint; subsequent tests must also pass the final gate.
- Fresh `loopd` and `loopctl` binaries build with all features, and the actual
  Provider host compiles. Rust protocol, installed CLI workflow,
  state/concurrency/recovery and full workspace/remote checks remain **pending**.
- No live model supplier, paid market-data download, production migration or
  formal factor admission is established by synthetic fixtures.

Review also identified two deadline cases now covered by regression fixtures:
an expired queued numerical child must use recovery rather than repeatedly fail
lease acquisition, and a transaction must recheck its clock before writing and
before commit. The latter rolls back all state/receipt/audit writes on clock
regression or a newly crossed parent deadline. These cases passed at `bf02ea0`.
The final writer also rechecks owner, numerical-reader and new-child authority;
a real RuntimeAuthority expiry fixture passed at that boundary.

The storage 2/4/8-process and kill/restart cases exercise terminal parent
advancement and transactional history. A separate installed CLI test races
2/4/8 independent processes at the actual Discovery-to-evaluation boundary and
checks one model call, one child, one reservation, one registered queued trial
and no numerical results.
Neither fixture establishes multi-host execution. The storage process/crash
matrix passed at `bf02ea0`; the installed CLI race still requires the corrected
numerical reservation path below.

The targeted `bf02ea0` run passed 57 cases, failed four installed workflows and
skipped one explicitly invoked subprocess helper. PR CI `38272618414` passed
TypeScript, Python protocol, Python research and legacy regression. The Rust,
unified-workspace and DaoCloud gates failed on the same four workflows plus one
historical-reader test. Inspection identified two independent causes:

- Generic numerical child reservation reused the model-call duration validator,
  incorrectly rejecting a valid 180-second numerical job at its 120-second
  ceiling. Numerical reservation now validates the general JobBudget duration
  and rounds up fractional milliseconds. Discovery retains its original model
  ceiling. New tests cover exact remaining time, overflow, invalid duration,
  unchanged reservations on denial and the unchanged Discovery ceiling.
- The historical-reader fixture authorized `run.1` while its job belonged to
  `run.fixture`. The fixture must explicitly grant that run and retain a
  negative ungranted-scope case; production authorization remains unchanged.

Fresh installed binaries, the corrected tests and replacement remote gates are
required before completion. These failed runs are not acceptance evidence for
the full numerical workflow.

After the numerical reservation correction, the installed CLI roundtrip passed
on the default thread stack in 206.13 seconds. It completed actual candidate
generation, Python evaluation/portfolio execution, process restart, receipt
replay and duplicate filtering. This focused pass precedes the final guard
changes and does not replace their regression or the complete remote gate.

The final start and advance writers also recheck their retained file guards
after database writes and immediately before commit. Start now repeats deadline
and live owner/child checks at that boundary. `start_guard_rollback` resolves a
real numerical recipe, replaces its file during the final clock check and
requires all job/run/transition/receipt/audit inserts to roll back. Restoring the
bytes does not revive the stale proof; fresh resolution must allow the same
unconsumed run ID and request key to start. The final targeted execution passed
this case, both installed scope/coverage fixtures, the installed roundtrip and
the historical-reader fixture. It passed 45 tests, failed the installed writer
fixture and skipped its explicitly invoked process helper.

The writer fixture expected zero trial rows after creating a queued evaluation
job. That contradicts the existing submission invariant: the real attempt is
registered in the same transaction as its job, before numerical execution, so
failed or cancelled attempts cannot disappear. The fixture now requires exactly
one trial bound to the queued child, factor and run, and still requires zero
evaluation/backtest results. No production behavior changes for this correction.
Its next run passed the two-process case but found no successful advancement
with four writers. A diagnostic rerun passed two and four writers, then all
eight clients reported transport failure. These are failed acceptance runs,
not evidence of a delivered concurrency claim.

Inspection found that every numerical step constructed a cold resolver and
rehashed the complete installed environment under the same bounded verification
deadline. The deployment now retains one root-bound NumericalResolver and
reuses its existing bounded FileVersion cache. Every operation still verifies
the plan, derives a fresh proof and checks current files and live authority;
no deadline or permission was relaxed. Tests cover shared resolver identity,
distinct proofs, wrong-root denial, same-length content changes, atomic leaf
replacement/deletion and root replacement/permissions. Installed fixture
failures now include a bounded warning log and the supplier-call count.
Final behavioral, lint and remote gates for this correction remain pending.

Independent review also found a real clock-ordering race in RuntimeAuthority:
it sampled before locking the watermark. A preempted caller could publish its
earlier sample after another caller and incorrectly report clock regression.
Sampling now occurs under the same lock as comparison/publication. A controlled
clock asserts that lock ownership precedes every sample, then exercises eight
threads; a separate regression case retains actual clock-rollback denial and
recovery. These tests require no timing sleeps and fail under the old ordering.
This source-level defect is not claimed as the sole cause of the earlier
installed transport errors; the corrected installed matrix must pass.

The combined correction builds fresh all-feature workspace binaries and passes
the complete all-target/all-feature Clippy gate with `-D warnings`, Rust format
checks, nine Rust run-boundary tests and all 4,965 naming declarations. The
updated local installed/concurrency/recovery execution and replacement remote
CI are pending at publication; the PR remains a draft until those gates pass.

The initial PR CI found stale current-descriptor metadata in
`wire_fixtures.json` and a missing generator-produced trailing blank line in
the TypeScript binding. These were corrected without changing the historical
baseline or relaxing comparisons; replacement CI remains required.
The complete local `scripts/proto-check.sh` now passes, including regenerated
bindings, pinned wire producers, breaking-change checks and role boundaries.

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
