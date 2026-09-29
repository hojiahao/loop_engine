# Phase 10 unit 2: Durable discovery model execution

Status: implementation and local acceptance passed; publication and exact-commit
remote CI remain delivery gates. No paid supplier or production database is used
by this acceptance.

## Requirement and design

A pinned research plan can request one text-to-AST model invocation through the
existing Provider service. Before outbound traffic, PostgreSQL reserves the full
input/output token and exact USD ceilings and records immutable request identity.
Only the winner of a lease/revision-fenced dispatch transaction may generate.
Recovery after that boundary is lookup-only, including an ABSENT Provider result.
Reservations survive cancellation, process death, network errors and ambiguity.

The budget scope is one job/one step, not an account-wide or multi-job run quota.
The fixed candidate schema and installed registry enforce numerical expression
syntax, but no numerical quality, checker review or factor admission is inferred.
Tools, multi-turn conversations and lifecycle controls remain subsequent units.

The Discovery API shares the existing authenticated listener and PostgreSQL job
aggregate. It receives no raw invocation or caller-selected URI. An administrator
pins actor/run, model snapshots, provider policy, prompt, schema, protocol,
registry and development-data manifest. The research-plan policy is separate
from the Provider deployment policy; a real Provider rejects substituting one
for the other. Rust independently canonicalizes the returned expression.

Migration 0011 adds immutable model-step evidence and an old-writer guard to
tracked jobs. Generic lifecycle commands cannot silently mutate a paid attempt.
No database transaction spans a Provider network request.

## Acceptance cases

- Cross-language request digest golden: Rust protobuf JSON must produce exactly
  the Provider journal identity, including timestamps, optional presence, byte
  encoding, enum names and 64-bit integer strings.
- Frozen plan and codec negatives: unsupported schemas/models/messages, template
  drift, malformed files, invalid AST nodes, budget excess and response mismatch.
- Real PostgreSQL storage: atomic reservation, replay, no repeated dispatch,
  conservative uncertainty, lease/revision/clock fencing and response integrity.
- Independent 2/4/8 OS writers plus kill/restart before and after reservation,
  dispatch and completion transactions.
- Actual mTLS Discovery service and compiled TypeScript Provider with a local
  synthetic HTTP supplier: candidate generation/read/restart, authenticating the
  owner, denied plan, corrupt or protected data, invalid candidate and lookup-only
  recovery after both services restart.
- Actual deployment JSON: enabled discovery configuration loads, while mismatched
  roles and overlapping plan/data namespaces deny startup.
- Recovery RPC deadlines respect both the original invocation budget and the
  Provider's five-second lookup ceiling, including sub-five-second deployments;
  the remaining lease always reserves time for evidence commit.

## Observed results

On 2026-09-29, the complete `just check` passed: Rust formatting and workspace
Clippy with `-D warnings`, protocol/source/fixture compatibility, function naming,
actual egress socket tests, TypeScript formatting/lint/type checks and Python
formatting/lint/types across the independently locked environments.

The first real integration pass exposed a recovery deadline mismatch, now fixed
without relaxing Provider limits. The initial storage pass exposed an invalid
test model: a codec-only uint64 boundary golden exceeded the Job contract's
context-window limit. The storage fixture now uses a valid model; the original
cross-language golden is unchanged. Final behavioral regression and remote CI
are recorded separately below.

The final serial Rust regression passed **72 tests, zero failures**, with one
ignored child-process entry point that the independent-process tests invoke
explicitly. The 71.86-second run covered all 12 actual Discovery/Provider cases,
all model-step storage cases (including 2/4/8 independent writers and six
before/after-commit crash cuts), codec goldens, frozen-plan negatives, deadline
limits and existing Discovery isolation cases. The prior readiness timeout did
not recur when compilation and service tests ran serially.

Additional contract checks passed: 12 Python discovery/wire cases, 10 TypeScript
discovery/wire cases and the Provider request-digest golden. `just check` also
regenerated and compared the committed cross-language artifacts. Full-workspace
tests and the clean DaoCloud container are mandatory remote CI gates; this local
targeted run is not a claim that the remote jobs have already passed.

## Reproduction

```bash
./scripts/pnpm.sh --filter @loop-engine/providerd build
bash scripts/postgres-test.sh start
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 ./scripts/cargo.sh test --locked --offline -p loopd --lib store::model_step -- --test-threads=1
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 ./scripts/cargo.sh test --locked --offline -p loopd --lib runtime::model_codec -- --test-threads=1
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 ./scripts/cargo.sh test --locked --offline -p loopd --lib runtime::discovery -- --test-threads=1
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 ./scripts/cargo.sh test --locked --offline -p loopd --lib runtime::service::discovery -- --test-threads=1
bash scripts/postgres-test.sh usage
bash scripts/postgres-test.sh stop
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CI=true just check
```

The dedicated PostgreSQL fixture is disposable. Do not point these tests at a
production database. Test child processes and temporary directories are scoped
to this project and removed at completion; unrelated `/tmp` content is untouched.

## Recovery and rollback limits

Before the absolute job deadline, an expired lease can be taken over without
altering the original request. After dispatch, no automatic paid resend occurs.
After the deadline, the historical projection remains readable and its reserve
remains held; this unit does not invent an extended execution budget. Broader
manual reconciliation and lifecycle operations belong to the later lifecycle unit.

To roll back, stop new discovery writers and disable the optional deployment
configuration. Preserve PostgreSQL model_steps/jobs/receipts/audit and the Provider
journal. Do not drop the new table, reset a job to queued or release uncertain
reservations. Existing immutable evidence remains available for later recovery.
Use a migration-aware binary with Discovery disabled: an older binary can reject
the new migration. No destructive down-migration or old-writer compatibility is
claimed.
