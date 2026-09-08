# Phase 3: Durable state checkpoint

- Date: 2026-09-08
- Owner: hojiahao
- Phase status: `in_progress`, not a completed Phase 3 exit gate.

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
is validated in all three languages;
the original Phase 2 descriptor trust anchor remains unchanged. Full amendment
host and remote CI results are recorded after their gates finish.

## Quality rules

`AGENTS.md` now makes the user's quality requirements persistent. Handwritten
`loopd` forbids unsafe code and the store denies missing public documentation.
The existing CI enforces rustfmt and Clippy with warnings denied, along with
cross-language and legacy regression gates. Public Rust guidance is a reference,
not certification against Microsoft, Google, Meta, Amazon, Tesla, or financial
institutions' private standards.
The repository naming guide favors concise domain verbs and focused behavioral
tests. It deliberately does not invent a universal name-length limit.

## Remaining Phase 3 gates

- Persist holdout approvals and monotonic period/grant records; atomically
  consume one grant and create every frozen-plan batch job, receipt, and event.
- Complete the backend-independent interface for those remaining aggregates.
- Pass final host, clean-container, and remote CI on the entire Phase 3 code;
  commit, push, and record closure evidence before marking Phase 3 complete.

No paid data, LLM call, research freeze, holdout unlock, production run, or
performance conclusion is authorized or performed by this checkpoint.
