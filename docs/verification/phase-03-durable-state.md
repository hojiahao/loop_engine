# Phase 3: Durable state checkpoint

- Date: 2026-09-08
- Owner: hojiahao
- Phase status: `in_progress`, not a completed Phase 3 exit gate.

## Implemented behavior

- SQLx checksum migrations; SQLite WAL, FULL sync, foreign keys, strict tables,
  indexed job projections, bounded busy/acquire/migration waits.
- A private pool behind `JobRepository`; database handles never enter provider
  or numerical worker code. SQLite is supported only on local filesystems.
- Atomic job mutation, revision CAS, immutable receipt, canonical audit append,
  and persisted server-clock watermark in `BEGIN IMMEDIATE`.
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

## Behavioral evidence

Host checkpoint gates completed on 2026-09-08: `just check`, `just test`,
`just build`, and `just doctor`. Rust: 102 passed plus two helper entry points
explicitly exercised by parent subprocess tests. Python protocol: 240 passed;
research: 1 passed; legacy: 216 passed, 1 skipped, 11 existing NumPy warnings.
TypeScript protocol/provider tests passed. Final clean-container and remote CI
evidence is still required; this checkpoint is not phase closure.

`durable_submission` covers migrations, reopen, checksum changes, actor-scoped
duplicates, semantic conflicts, protocol availability, bounded/cancellable
migration locks, clock regression, audit failure rollback, envelope/projection
corruption, immutability, and SQL constraints.

`durable_lifecycle` covers revision races, lease ownership, exact expiry,
deadline caps, heartbeat replay, stale-worker completion, cancellation, terminal
outcomes, malformed payloads, receipt failure rollback, and restart replay.

`durable_processes` starts 2, 4, and 8 independent OS processes, each with its
own pool, for duplicate submission, distinct submission, and lease CAS races.
Ready/start barriers ensure overlap. Child exit and waits are bounded; child
guards kill/reap processes on failure. Every result is checked against the
persisted event chain.

Library crash tests SIGKILL a real child at three deterministic points:
after job/receipt/audit writes but before COMMIT, after COMMIT but before the
response, and after lease acquisition. Reopening verifies rollback or durable
replay, configuration, and explicitly failed abandoned work. Fault hooks exist
only under `cfg(test)`, never in a production executable. Two ignored helper
tests are invoked explicitly as child processes by these parent tests; they are
not omitted behavioral scenarios.

## Quality rules

`AGENTS.md` now makes the user's quality requirements persistent. Handwritten
`loopd` forbids unsafe code and the store denies missing public documentation.
The existing CI enforces rustfmt and Clippy with warnings denied, along with
cross-language and legacy regression gates. Public Rust guidance is a reference,
not certification against Microsoft, Google, Meta, Amazon, Tesla, or financial
institutions' private standards.

## Remaining Phase 3 gates

- Complete role-owned submission mapping and authorization integration.
- Persist holdout approvals and monotonic period/grant records; atomically
  consume one grant and create every frozen-plan batch job, receipt, and event.
- Complete the backend-independent interface for those remaining aggregates.
- Pass final host, clean-container, and remote CI on the entire Phase 3 code;
  commit, push, and record closure evidence before marking Phase 3 complete.

No paid data, LLM call, research freeze, holdout unlock, production run, or
performance conclusion is authorized or performed by this checkpoint.
