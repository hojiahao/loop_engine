# ADR 0006: Durable state and command transactions

- Status: Accepted
- Date: 2026-09-07
- Owner: hojiahao

## Decision

`loopd` owns a SQLite-backed repository behind a storage-independent Rust
interface. Provider and research processes never receive database handles.
Immutable data stays in artifact storage, not metadata rows or RPC bodies.
PostgreSQL will implement the same command semantics, not SQLite SQL strings.

The default database uses WAL, FULL synchronous mode, foreign keys, bounded
busy/acquire timeouts, strict tables, and forward-only checksum-verified SQLx
migrations. An OS advisory lock serializes startup migrations for one local
database. Normal writes use SQLite transactions, not a process-only lock.
Network filesystems and shared SQLite deployments across hosts are unsupported.

Each accepted command commits its aggregate revision, immutable idempotency
receipt, and canonical append-chain audit event in one `BEGIN IMMEDIATE`
transaction. Unique keys prevent duplicate jobs or receipts across processes.
An identical command returns its original response without another revision or
audit event. Reusing its scoped idempotency key with different semantic content
is a conflict. Stored Protobuf bytes are decoded and compared as typed values;
their byte order is not an idempotency or research identity. Byte checksums are
used only to detect corruption of the original stored envelopes.

All updates require an expected revision. Worker updates additionally require
the current lease ID, authenticated owner, and an unexpired half-open lease
interval. Heartbeats cannot revive expired leases or exceed the frozen job
deadline. The server clock is sampled after acquiring the write transaction;
backward time relative to the persisted clock watermark fails closed.

Queued cancellation or wall-clock exhaustion has attempt zero. Lease
acquisition increments the actual execution attempt; no artificial attempt is
created to represent administrative cancellation. Running, successful,
factor-rejected, and infrastructure-failed records require a positive attempt.
This is a validation correction with no wire-field or descriptor change.
Before exposing the behavior to mixed-version workers, negotiation must
require the corresponding job-lifecycle capability.

Expired work is not silently re-executed. Recovery records a typed
infrastructure failure, or budget exhaustion when the absolute job deadline
has passed. Later Run Harness policy may retry eligible work using explicit
lineage and external idempotency. Transactional commit-once is not a claim of
exactly-once external model calls or arbitrary tool execution.

Holdout approval and consumption reuse the same transaction boundary. The
eventual consume command must advance the grant and period, create the batch,
insert all frozen-plan jobs, append audit evidence, and store its receipt
atomically. No partial batch can be externally visible. Capability issuance
and sample-role resolution remain separate mandatory Phase 4/5 gates.

## Admission and authority

The repository accepts only structurally valid jobs after a server-owned
admission policy approves their references and frozen protocol selection.
The default policy denies submission. Fixture policies exist only in tests.
Phase 3 does not expose an unauthenticated generic enqueue or SQL endpoint.
Future role handlers authenticate transport principals, resolve opaque
references, and pass trusted commands; caller-supplied actor labels alone
never grant authority. A valid wire DTO is not proof of authorization.

Audit IDs are independent of research identities. Audit payload and event
hashes use `loop-core` canonical writers. SQL triggers reject updates and
deletes of audit rows and command receipts. The chain detects corruption but
does not protect against an administrator rewriting the database and every
hash; external immutable anchors belong to the later audit deployment policy.

## Exit Evidence Required

- Migration checksum, WAL, synchronous, foreign-key, constraint, and reopen tests.
- Multi-connection and multi-process duplicate submission and revision races.
- Lease owner fencing, expiry boundaries, heartbeat, cancellation, deadline,
  backward-clock, and terminal-outcome tests.
- Kill/restart tests before commit, after commit, and while a lease is active.
- Command/audit atomicity and tamper detection, including replay after restart.
- Protocol availability and default-deny admission tests.
- Atomic all-or-nothing holdout batch tests and role-handler integration.
- Host commands, clean container, pushed commit, and successful remote CI.

## References

- [SQLite transactions](https://www.sqlite.org/lang_transaction.html)
- [SQLx 0.8.6 pools and transactions](https://docs.rs/sqlx/0.8.6/sqlx/struct.Pool.html)
