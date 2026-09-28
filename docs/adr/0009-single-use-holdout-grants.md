# ADR 0009: Single-use holdout grant lifecycle

- Status: Accepted for the Phase 3 storage boundary
- Date: 2026-09-08
- Owner: hojiahao

## Purpose and authority

A grant records permission to evaluate a complete, already frozen candidate
batch against one protected historical period. It is not a trading permission,
a general enterprise approval workflow, or a bearer secret. Daily IS and
development searches do not use this workflow. Personal deployment may pin a
policy requiring one authenticated human confirmation; a policy requiring more
than one approver must resolve genuinely independent identities. Two fixture
approvers in tests do not set a production minimum.

The server-owned policy authorizes each operation and resolves the exact freeze,
its pinned approval policy, complete canonical plan, and owning BacktestSpec
documents from bounded, pre-resolved metadata. No network I/O is allowed under
the ledger lock. The repository independently reparses period and plan identities,
checks all bindings, and bounds metadata to eight MiB of plan and 64 MiB of
BacktestSpec artifacts. The ordinary `ResolvedFreeze` value conveys no authority.
Fixture registries are not production freeze or BacktestSpec implementations.
Production policies continue to deny and no mutating transport endpoint is added.

## Issuance and terminal transitions

Forward-only SQLx migration `0004_holdout_grants.sql` adds grant records and an
immutable approval attachment relation. Issuance requires the expected sealed
period revision, one to eight unexpired persisted human approvals, distinct
actor IDs and authenticated subjects, and the frozen policy's count and roles.
The validity interval is half-open, positive, at most seven days, and no longer
than any attached approval. Issuance advances the period and atomically inserts
the grant, attachments, immutable receipt, audit event, and clock watermark.

The period has at most one grant, including after expiry or revocation. A grant
is either issued revision one or terminal revision two. Expiry closes at or
after its original expiry time; revocation closes before expiry with an explicit
reason. Both advance the period to closed revision three while retaining its
original grant identity. Closing is independently authorized and does not depend
on freeze artifact availability, so an artifact outage does not prevent revocation.
An elapsed grant may remain in the issued lifecycle state until an explicit
expiry command runs; reads are historical metadata, not a validity decision.
The future batch consumer must check time and state transactionally.

PostgreSQL constraints enforce monotonic revisions, immutable identities, unique
period and approval attachment, and distinct attached actors and subjects.
Deferred constraint triggers require the final period, grant, and attachment
count to agree at commit. Null terminal timestamps cannot satisfy terminal-state
checks. Rust additionally verifies canonical approvals, complete wire records,
checksums, every indexed projection, and time/plan bindings on reads and replay.
Grant reads use one repeatable-read snapshot to avoid a torn aggregate during
concurrent close. Database-owner corruption is detected, not silently repaired.

## Replay and failure behavior

The authenticated actor, operation, and idempotency key scope each receipt.
Transport request ID and requested time may change on retry; other semantic
inputs must not. An issuance retry returns the original issued response even
after expiry or revocation, without renewing, reopening, or authorizing data
access. Current state is retrieved separately. Closing retries return their
original terminal result and cannot change the recorded reason.

The existing PostgreSQL ledger row serializes writers across OS processes, with
bounded database and lock waits. Clock regression, invalid state, stale revision,
unresolved authority, corruption, cancellation, audit failure, and receipt failure
fail closed. A transaction that has not committed rolls back completely. A retry
with the same key resolves an uncertain commit outcome.

## Remaining gates

This checkpoint implements issuance, authorized reads, expiry, and revocation.
Atomic consumption of the complete frozen plan and insertion of all batch jobs
remain a separate Phase 3 gate. Consumed protocol/database states alone are not
that implementation. Production authorization, protected artifact retrieval,
freeze schemas, numerical BacktestSpec semantics, and automatic runtime expiry
scheduling belong to later owning components. No real grant or data unlock is
created during this checkpoint. Phase 3 remains open until its complete gate,
commit, push, and remote CI have passed.
