# Phase 3: Durable state verification

- Date: 2026-09-09
- Owner: hojiahao
- Phase status: `complete`; implementation and closure evidence are below.

## Implemented behavior

- SQLx checksum migrations; PostgreSQL TLS, synchronous commits, foreign keys,
  checked/indexed job projections, bounded lock/acquire/migration waits. The
  original SQLite backend is superseded by ADR 0007 and retained only as history.
- A private pool behind `JobRepository`; database handles never enter provider
  or numerical worker code. Administrative migrations use a separate connection
  and authority; normal startup verifies schema without DDL.
- Atomic job mutation, revision CAS, immutable receipt, canonical audit append,
  and persisted server-clock watermark in a PostgreSQL transaction. The ledger
  row is locked before sampling time, serializing accepted ledger mutations.
- Scoped semantic idempotency. Replays return the original record; they do not
  increment revisions, extend leases, or authorize another dispatch.
- Acquire, heartbeat, complete, cancel, and scheduler-only expired-job recovery.
  Lease intervals are half-open, capped at the frozen deadline, and fenced by
  lease ID plus authenticated owner. The first heartbeat acknowledges running.
- Expired leases produce typed, non-automatically-retryable infrastructure
  failures. Absolute deadlines produce budget exhaustion, including attempt-zero
  queued work. Recovery is not an exactly-once external-effects guarantee.
- Shared Rust/TypeScript/Python job-record vectors increased from 105 to 109
  without changing Protobuf fields or the Phase 2 descriptor trust anchor.
- `loopd` opens and verifies durable state before listening; `/readyz` reports
  storage readiness. Mutating RPCs remain unregistered and policies default deny.
- Role-owned Discovery, Factor Evaluation, Backtest, and Reconciliation requests
  map into the same transactional store. IDs and acceptance times are assigned
  after receipt lookup inside the write transaction. Retries retain the original
  role-specific handle and pinned protocol, even after a new negotiation.
- Canonical sealed-period registration behind independent protected-state
  authorization. Period, immutable replay receipt, clock watermark, and the new
  typed period audit target commit atomically. Registration does not issue an
  approval, grant, data capability, or research job.

## Behavioral evidence

Initial storage checkpoint (`53a0ab2`) host gates completed on 2026-09-08: `just check`, `just test`,
`just build`, and `just doctor`. Rust: 102 passed plus two helper entry points
explicitly exercised by parent subprocess tests. Python protocol: 240 passed;
research: 1 passed; legacy: 216 passed, 1 skipped, 11 existing NumPy warnings.
TypeScript protocol/provider tests passed. The container-source correction
`cdb1d16` passed all seven jobs in GitHub Actions run `34183705775`, including
the clean-container gate. This checkpoint is not phase closure.

The role-submission checkpoint passes the same host commands, followed by final
workspace-wide Rust tests and Clippy after the receipt integrity checks were
added. Rust: 120 passed plus the two exercised subprocess entry points; Python
protocol: 240 passed; research: 1 passed; legacy: 216 passed, 1 skipped, with
existing NumPy warnings. Commit `b8b63d7` passed all seven jobs in GitHub Actions
run `34185934175`. These are historical SQLite checkpoint results, not evidence
for the PostgreSQL port.

`role_submission` has 18 focused tests covering all four input mappings and
safe projections, restart replay, server-assigned identity/time, frozen protocol
retention, concurrent retries, actor mismatch, changed semantics/run, unknown
references, unavailable protocols, default-deny policy, and audit rollback.
Correctly rehashed but impossible receipt states and cross-linked receipt rows
are rejected. These use explicit fixture admission policies, not a claim that
production authentication or artifact/model registries are implemented.

`durable_submission` covers migrations, reopen, checksum changes, actor-scoped
duplicates, semantic conflicts, protocol availability, bounded/cancellable
migration locks, clock regression, audit failure rollback, envelope/projection
corruption, immutability, and SQL constraints.

`durable_lifecycle` covers revision races, lease ownership, exact expiry,
deadline caps, heartbeat replay, stale-worker completion, cancellation, terminal
outcomes, malformed payloads, receipt failure rollback, and restart replay.

`durable_processes` starts 2, 4, and 8 independent OS processes, each with its
own pool, for duplicate submission, distinct submission, lease CAS races, and
role-owned submission retries.
Ready/start barriers ensure overlap. Child exit and waits are bounded; child
guards kill/reap processes on failure. Every result is checked against the
persisted event chain.

Library crash tests SIGKILL a real child before and after COMMIT for both trusted
internal and role-owned submissions, and after lease acquisition. Reopening verifies rollback or durable
replay, configuration, and explicitly failed abandoned work. Fault hooks exist
only under `cfg(test)`, never in a production executable. Two ignored helper
tests are invoked explicitly as child processes by these parent tests; they are
not omitted behavioral scenarios.

## PostgreSQL amendment

On 2026-09-08 the owner authorized a PostgreSQL-only runtime and provisioning on
the designated production host. Read-only preflight found PostgreSQL 17.11
already installed with TLS and SCRAM authentication. No server upgrade, restart,
public listener, firewall change, or unrelated database mutation was performed.

Provisioned database `loop_engine`, non-login schema owner `loop_engine_owner`,
and restricted runtime login `loop_engine_app`. Migration versions 1 and 2 were
applied using the administrative bundle. Application login negotiated TLSv1.3.
Read-only privilege checks confirmed no superuser, database/role creation,
replication, bypass-RLS, schema CREATE, migration INSERT, or audit DELETE rights.
The actual Rust `loopd --check-database` executable passed schema checksums,
ledger identity, and TLS/session verification through the SSH tunnel. The
connection file is Git-ignored and mode 0600; its value is not published.

The application requires `sslmode=require` or stronger. This is encryption, not
a claim of certificate-name validation for the current self-signed certificate.
The SSH host key authenticates the remote hop. Shared server HBA policy remains
unchanged. Runtime command policies still default deny.

All destructive tests target a separate DaoCloud PostgreSQL 17.11 test container
with ephemeral TLS and per-fixture schemas, never production. The fixture
rejects non-test database/login names. A first host attempt under compilation
load hit the migration deadline while the test server was restarting; the gate
remained failed until rerun. Container memory counters showed no OOM events.
This observation is not represented as a production fault or a proven cause.
A later two-schema test exposed SQLx's redundant database-wide migration lock.
The wrapper now uses only its schema-scoped lock, including namespace creation,
to avoid serializing unrelated schemas. A dedicated regression holds SQLx's
database-wide lock while opening an independent schema. Runtime ledger-row
locking and query deadlines are unchanged.

`durable_holdout` adds 15 tests: both period roles, authorized reads, canonical
identity, actor spoofing, default denial, future/retrograde time, immutable
history, receipt and audit rollback, correctly rehashed corrupt receipts,
restart replay, semantic conflict, duplicate keys, and terminal reset denial.
The process and crash matrices also include period registration, with real
clients killed immediately before and after commit. Lifecycle advancement in
the terminal-reset fixture is test-only SQL, not an implemented grant API.

`postgres_configuration` adds seven tests for explicit TLS, closed URL syntax,
redacted errors, hostile schema names, no implicit runtime DDL, and migration
checksum tampering, and schema-scoped migration isolation. The audit extension
is validated in all three languages; the original Phase 2 descriptor trust anchor
remains unchanged.

Implementation commit `d85ae710cbb9fd8ac9e1fd3ba8e8dc816985fd1f` is pushed.
[GitHub Actions run 34200778090](https://github.com/hojiahao/loop_engine/actions/runs/34200778090)
passed all seven jobs: Rust, TypeScript, Python research/protocol/legacy,
unified workspace gates, and the clean DaoCloud development container. The Rust
job passed 142 tests plus two helper entry points invoked by parent subprocess
tests. This includes the new migration-isolation regression, all 18 role tests,
15 period tests, 2/4/8-process matrices, and kill/restart boundaries.

Local `just check`, final Clippy, TypeScript checks/tests, Python protocol tests
(240), research tests (1), `just build`, and `just doctor` passed. The final
production `loopd --check-database` probe also passed. Full local test reruns were
stopped after successful CI to avoid further low-memory linking/test overhead;
they are not counted as a completed local full-suite gate. An earlier local
legacy run lost its temporary directory; a later isolated run reported 216
passed and 1 skipped but was interrupted during process teardown. The completed
legacy gate is the successful CI job, not that interrupted local invocation.

No password was committed: the committed-file scan checked the configured secret
and its percent-encoded form across all 58 changed files before the SSH push.
Docker excludes host runtime secrets from both build context and development
container mounts. The checkpoint is verified; the Phase 3 exit remains open.

## Human approval storage checkpoint (2026-09-08)

ADR 0008 adds immutable human approvals under the existing SQLx framework and
forward-only PostgreSQL migration 3. The record, original retry receipt, audit
append, and clock watermark commit atomically. The default resolver denies;
no protected RPC, real approval, grant, data unlock, or production schema change
was performed by this checkpoint. Migration 3 remains a pending administrative
deployment, not a runtime startup operation.

The initial sandboxed test execution failed because network socket creation was
denied, before it could connect to PostgreSQL. The authorized rerun used only
the disposable `loop_engine_test` database and passed. Local evidence:

- `durable_approval`: 19 passed, including canonical byte/digest golden, full
  persisted attribution, original-expiry replay after restart, semantic
  conflicts, independent human receipts, default denial, spoofed/non-human
  principals, unresolved references, bounds, clock regression, expiry bounds,
  audit/receipt rollback, immutability, rehashed tampering, lifecycle gating,
  authorized reads, and cancellation while waiting for the ledger lock.
- `durable_holdout`: 15 passed, preserving existing period behavior.
- `postgres_configuration`: 7 passed.
- Library tests: 2 passed, including real kill/restart before and after approval
  commit; its ignored helper is explicitly invoked by the parent test.
- `durable_processes`: parent matrix passed for 2/4/8 independent writers,
  including same-key approval replay and distinct-key approval commits. Its
  ignored worker is explicitly invoked, not omitted concurrency coverage.
- `just check`: passed protocol/producer conformance, formatting, Clippy with
  warnings denied, TypeScript checks, and Python Ruff/mypy checks.

The approval golden was independently calculated with Node's JSON serializer
and SHA-256 implementation; the Rust database test verifies every canonical
byte, with only the generated approval ID substituted, and the raw digest.
This is not a claim that TypeScript/Python approval hash APIs or production
freeze/BacktestSpec registries have been implemented.

Approval checkpoint `47e8128` is pushed. All seven jobs in GitHub Actions run
`34209334491` passed, including the unified workspace and clean DaoCloud container
gates. This evidence validates that checkpoint, not later working-tree changes
or full Phase 3 closure.

## Grant lifecycle checkpoint (2026-09-08)

ADR 0009 and forward-only migration 4 add one grant per period, immutable human
approval attachments, issued/terminal revisions, authorized reads, expiry, and
revocation. The independent production policy remains default-deny. No protected
transport endpoint is exposed and migrations 3/4 are not deployed to production.
Frozen-plan batch consumption is still pending.

Local command:

```bash
CARGO_BUILD_JOBS=1 ./scripts/cargo.sh test -p loopd --locked \
  --test durable_grant --test durable_approval --test durable_holdout \
  --test durable_processes --lib -- --test-threads=1
```

Result: 63 passed. The two ignored subprocess entry points are explicitly run by
their parent process/fault tests, not skipped behavior. Breakdown:

- 26 grant tests: exact frozen bindings, one-person policy, independent subjects,
  duplicate actors, required count, expiry boundaries, immutable replay after
  restart or terminal closure, default denial, spoofing, clock regression,
  unresolved/corrupt plan bytes, audit/receipt rollback, cancellation, rehashed
  receipt tampering, and deferred database aggregate constraints.
- 19 approval and 15 period tests pass. The two tests formerly advancing a
  simulated grant with direct SQL now use real issue/revoke repository commands.
- Two library tests pass, including grant and close kill/restart before and
  after commit, alongside the existing job/role/period/approval crash cases.
- One process-matrix parent passes 2/4/8 independent writers for same-key grant
  and close retries, and different-key races with one commit and fenced losers,
  alongside all previous process modes. Audit chains remain valid.

Full `just check` passed: cross-language protocol checks, rustfmt, workspace
Clippy with warnings denied, TypeScript format/lint/typechecks, Python 3.14.4
environment verification, Ruff, and mypy. Commit `eb95170` is pushed; all seven
jobs in GitHub Actions run `34303072037` pass, including the unified workspace
and clean DaoCloud container. This evidence does not cover the later batch work.

After tests, the disposable PostgreSQL container and its tmpfs test database
were removed. At the owner's request, stale root `target/`, unused uv and Docker
build caches, protocol-generation staging directories, checker caches, and Rust
incremental caches were cleaned. Source, Git history, research records, secrets,
installed toolchains, the root `.venv`, and reusable compiled dependencies remain.
These are reconstructible artifacts, not research-data deletion. Docker-reported
active build-cache leases were not forcibly removed.

## Atomic batch implementation (2026-09-09)

ADR 0010 and forward-only migration 5 implement complete frozen-plan consumption
behind `HoldoutRepository`. The narrow request cannot supply research inputs or
budgets. The protected schema resolver and separate job admission policy default
deny. No production jobs or data capabilities are created by this implementation.

The first full local test attempt found an invalid audit command identifier:
`loop.holdout.consume-grant` did not satisfy the canonical audit schema. The
unpublished operation was corrected to `loop.holdout.consume_grant`; the failed
attempt is not counted as a passing gate. The subsequent targeted command passed:

```bash
CARGO_BUILD_JOBS=1 ./scripts/cargo.sh test -p loopd --all-features --locked \
  --test durable_batch --lib -- --test-threads=1
```

- 20 batch tests passed: exact plan/budget mapping, immutable replay after
  expiry/restart, independent admission, actor and reference binding, terminal
  denial, rehashed receipt corruption, lifecycle-compatible replay, database
  membership constraints, parser outages, partial insertion and receipt rollback,
  cancellation, and clock/expiry changes during materialization.
- Two library tests passed. The crash parent explicitly invokes its ignored
  child entry point and kills clients mid-batch and before/after commit, in
  addition to the previous job, lease, period, approval and grant boundaries.
- `cargo clippy -p loopd --all-targets --all-features --locked -- -D warnings`
  passed before the identifier-only correction. Final workspace gates remain
  required for phase closure; no remote CI evidence covers these edits yet.

Implementation checkpoint `e8572cf` is pushed. The next full host attempt passed
the batch, approval, grant, period, and lifecycle cases, then failed in the
process matrix because the reused 256 MiB fixture tmpfs was full. Read-only
inspection found 183 test schemas, a 154 MB test database, and no free volume
space. This was not production or cloud-disk exhaustion. Managed full suites now
take a local fixture lock, recreate the dedicated test service, and remove it on
exit. Host and clean-container fixture limits are 512 MiB tmpfs / 640 MiB memory.
Explicit test URLs remain caller-owned. The failed run is not a passing gate.
GitHub Actions run `34309854814` passed TypeScript and all three Python jobs;
Rust, unified workspace and clean-container tests failed on the same fixture
`pg_wal` volume exhaustion, followed by connection/recovery errors. This confirms
the original fixture was too small even for a fresh expanded suite.

The first host run with the larger, fresh fixture passed the complete 2/4/8
process matrix and submission tests, but an independent-schema migration test
timed out. It ran while the cleaned Python dependency cache was being restored;
that overlap is recorded, not asserted to be the cause. Production deadlines
and tests were not relaxed. The isolated Python rerun completed with 216 passed,
1 skipped and 12 NumPy warnings. A second managed test invocation correctly
rejected the held fixture lock without disturbing the active suite; cleanup
removed the fixture even after the failed gate.

## Quality rules

`AGENTS.md` now makes the user's quality requirements persistent. Handwritten
`loopd` forbids unsafe code and the store denies missing public documentation.
The existing CI enforces rustfmt and Clippy with warnings denied, along with
cross-language and legacy regression gates. Public Rust guidance is a reference,
not certification against Microsoft, Google, Meta, Amazon, Tesla, or financial
institutions' private standards.
The repository naming guide favors concise domain verbs and focused behavioral
tests. It deliberately does not invent a universal name-length limit.

## Phase 3 closure (2026-09-09)

Implementation `e8572cf4716652b2541089834429036f600ebc58` and fixture fix
`aa3bd2b2153232a2b34a9d41b2723e2bf9c10776` are pushed. All seven jobs in
[GitHub Actions run 34311911291](https://github.com/hojiahao/loop_engine/actions/runs/34311911291)
passed: Rust, TypeScript, Python protocol/research/legacy, unified workspace,
and the clean DaoCloud development container. Earlier failed attempts above
remain part of the record, not substitutes for this successful gate.

The final host `just check`, `just test`, `just build`, and `just doctor` all
completed successfully. Host tests ran without simultaneous dependency restore:

- Rust: 207 passed; two ignored helper entry points were explicitly invoked by
  subprocess parents. The 2/4/8-process matrix passed in 87.88 seconds, and all
  seven PostgreSQL configuration tests passed, including schema-lock isolation.
- TypeScript: 59 protocol and 1 provider-skeleton tests passed.
- Python 3.14.4: protocol 240, research skeleton 1, legacy 216 passed / 1 skipped.
  The legacy suite reported 12 NumPy warnings; they are not hidden or promoted
  to evidence that the new numerical research implementation exists.
- The host fixture container and tmpfs were automatically removed on completion.

The administrative bundle applied forward-only production migrations 3, 4 and 5
in one transaction after CI passed. Read-only inspection confirms successful
versions 1 through 5 and zero jobs, periods, grants and batches. The restricted
runtime login can SELECT but not UPDATE batch records, UPDATE grant lifecycle
state, and cannot INSERT migration metadata. The just-built `loopd` executable
passed `--check-database`, verifying TLS/session policy and SQLx checksums using
the private mode-0600 connection reference. No credential appeared in output.

Phase 3 is complete at its storage boundary. Production transport identity,
holdout worker capabilities, trusted research/data registries, numerical
execution and model providers remain later-phase work. No production research
outcome, factor admission or sample-unlock claim follows from these tests.

No paid data, LLM call, research freeze, holdout unlock, production run, or
performance conclusion is authorized or performed by this checkpoint.

## Post-closure migration regression (2026-09-09)

After numerical checkpoint `7e6352a`, a full host regression failed during the
independent-process startup matrix. PostgreSQL rejected creation of
`pg_catalog._sqlx_migrations` with SQLSTATE `42501`. The intended target schema
did not exist when the migration session connected. PostgreSQL ignores missing
search-path entries and uses the first existing schema as the creation target
([PostgreSQL search-path rules](https://www.postgresql.org/docs/17/ddl-schemas.html#DDL-SCHEMAS-PATH));
the exact internal cache/snapshot timing of the observed race is not established.
The contemporaneous seven-job CI run `34315225076` passed, but does not invalidate
this local failure. No production data or published migration was changed.

Migration bootstrap now connects with only `pg_catalog`, acquires the existing
namespace lock, creates the target, explicitly changes the search path and
checks `current_schema()` before any SQLx migration metadata DDL. The regression
test waits for an actual advisory-lock blocker before creating the namespace
from another connection, then checks the migration table's real namespace and
the complete store configuration. Deadlines and migration checksums are unchanged.

The targeted 2/4/8-process matrix passed in 94.06 seconds, followed by all eight
PostgreSQL configuration tests. Full `just check` passed, including rustfmt,
Clippy with warnings denied, protocol boundaries and all language type checks.
The rebuilt `loopd --check-database` also passed against the production database
without DDL or research writes. The subsequent full host `just test` passed:
all Rust suites including kill/restart and the process matrix (86.66 seconds),
TypeScript 60, Python research 46, protocol 240, and legacy 216 passed / 1 skipped.
The legacy suite still reports 12 NumPy warnings. Managed fixture cleanup ran
successfully. Remote workspace and clean-container acceptance must use the CI
checks attached to this fix's commit, not the older green run.
