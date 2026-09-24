# ADR 0010: Atomic frozen-plan batch consumption

- Status: Accepted for the Phase 3 storage boundary
- Date: 2026-09-09
- Owner: hojiahao

## Decision

Implement the existing narrow `ConsumeGrantAndEnqueueBacktestRequest` in the
PostgreSQL repository. The request provides only authenticated command metadata,
an exact grant reference, and expected grant/period revisions. The handler's
`SubmissionMetadata` supplies a server-resolved run and negotiated protocol; no
caller-provided FactorSpec, BacktestSpec, budget, or alternative plan is accepted.
Production transport interception and protected resolvers remain default-deny.

The repository reauthorizes the principal, locks the ledger, verifies the current
grant aggregate, and reparses the complete frozen plan. Its owning schema parser
must materialize each exact referenced BacktestSpec document; the hook defaults
to denial. The repository independently binds the resulting factor ID, artifact
digest, exact sample and snapshots, configuration, source tree, and data manifest.
The owning parser must additionally validate numerical policy, registry, calendar,
environment and seed semantics. Synthetic storage fixtures do not implement that
Phase 7 research responsibility. Generic job submission still cannot enqueue a
holdout job, and the separate job admission policy must approve each derived job.

Budget integers and exact decimal costs are converted from the canonical plan;
no floating-point conversion or request override occurs. The ordered batch has
one job per plan entry, between one and 4,096 entries. Metadata is bounded to
64 MiB of generated job envelopes. The whole command has a 30-second deadline,
in addition to existing PostgreSQL statement and lock timeouts. Materialization
yields between bounded entries to permit cancellation. Policy hooks must remain
bounded, deterministic, side-effect-free, and free of network I/O.

## Atomic state and replay

SQLx migration `0005_holdout_batches.sql` adds immutable batches and ordered job
associations. One transaction creates the batch and every job, advances the grant
to consumed revision two and period to consumed revision three, appends each job
acceptance event plus the grant-consumption event, and stores the original receipt.
The grant's validity and each job's deadline are checked again before commit;
clock regression aborts and the highest observed clock is persisted.

Deferred PostgreSQL constraints require every consumed grant to have exactly one
complete batch. Links have unique job IDs and contiguous plan slots by combining
the primary key, bounded index, and exact batch count. Each linked job must be a
holdout job with the same run and submission time; a holdout job cannot commit
without membership. Link-level checks use indexed point lookups rather than
recounting the complete batch once per job. Immutable batches and links prevent
later additions, removal, or rebinding through the runtime account.

A same-key retry verifies its original request, response, grant, period, batch,
job order, immutable specification checksums, protocol availability, and freshly
resolved frozen entries. It retains the original protocol selection, does not
renew expired permission, and creates no new execution. Changing the server run
conflicts. Existing jobs may have advanced their execution lifecycle; their
specifications must remain identical. Reads use bounded individual job loads,
not a potentially 16 GiB join of 4,096 maximum-size envelopes.

Cancellation, deadlines, second-job insertion failure, audit/receipt failure,
materializer failure, corruption, and stale revisions fail closed. Incomplete
work is never visible outside the transaction. An uncertain commit is resolved
by retrying the same key. Database commit-once does not imply exactly-once
external model calls, file writes, or research execution.

## Verification and remaining boundaries

Exercise independent 2/4/8-process same-key retries and different-key races,
kill/restart during the first job insertion and before/after commit, database
constraints, cancellation, clock changes, rehashed corruption, and exact plan
budget mapping. Keep all existing storage, numerical, and cross-language tests.
Close Phase 3 only after complete host and clean-container gates, commit, push,
and remote CI evidence. Worker capability issuance, artifact reads, production
authentication, research schema semantics, and actual numerical execution remain
explicit Phase 4/5/7/10/11 responsibilities, not achievements of a storage test.
