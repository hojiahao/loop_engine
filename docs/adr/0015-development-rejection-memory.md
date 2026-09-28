# ADR 0015: Transactional development rejection memory

- Status: Accepted for the internal job path; production execution remains unavailable
- Date: 2026-09-10
- Owner: hojiahao

## Requirement And Decision

Finding 6 identified a failed-hash filter that production callers never used.
Connect rejection memory directly to the existing job insertion and lease
acquisition paths, not to an optional caller-supplied set. Both direct internal
submission and role-owned development backtest submission use this gate.

Migration 7 adds one immutable `backtest_rejections` projection of committed
development backtest outcomes. The source job retains the complete rejection,
reason, evidence and frozen inputs; the projection stores only its revision,
context digest, code and commit time. An indexed, single-row lookup avoids
decoding or scanning an unbounded history of Protobuf envelopes. No service,
dependency, RPC or generic memory framework is added.

The context uses the already canonical FactorSpec ID, ordered snapshot IDs,
data manifest, return definition, six provenance digests and deterministic seed.
Its versioned internal JSON lookup format is specified in
`docs/development/failure-memory.md`; Protobuf bytes are not hashed. Run IDs,
job IDs, budgets and retry metadata cannot bypass the filter. Different code,
operators, configuration, data, calendar, environment or seed does not inherit
an old rejection. This is exact frozen-context memory, not a permanent ban on
an expression across datasets or research regimes.

All development backtest rejections are preserved, but only insufficient
coverage, deterministic-filter and performance codes block a fresh submission.
Duplicate/previously-failed are not new empirical evidence. Correlation,
semantic-review and policy codes can depend on library/model/authorization
state absent from this context and are not reusable by this gate.
Infrastructure failures, cancellations, budget exhaustion, factor-evaluation
jobs and holdout jobs do not enter this projection. Factor-evaluation inputs
currently lack a complete execution fingerprint; do not invent one.

## Transactions And Authority

The lease-fenced completion handler inserts the projection in the same
transaction as the terminal job revision, immutable receipt and audit event.
Deferred constraints require exactly one matching projection per rejected
development backtest and prohibit projections for other outcomes. Updates and
deletes are denied. Replay verifies the original projection without inserting
another observation or rewriting history.

New submissions check memory after existing authorization/reference resolution
and under the same bounded ledger write lock used by completion. Acquisition
checks again after idempotent replay handling: a job queued before a rejection
cannot later start a redundant evaluation. A previously leased job is not
retroactively cancelled; concurrent already-started trials retain their own
evidence. This is not in-flight deduplication or general trial accounting.

`StoreError::PreviouslyRejected` means existing deterministic domain evidence,
not an infrastructure failure or a new `FactorRejection`. No historical job ID
or metric is returned. A denied new submission writes no job/receipt; a denied
acquisition leaves its job queued for explicit cancellation. Transport and
Loop Runtime must map this result to skip/cancel, not operational retries.
Those handlers are still pending. Default production reference policies still
deny; this change does not enable numerical execution or holdout access.

## Verification And Rollback

Verify both submission paths, restart/replay, each context dimension, operational
failures, expired leases, clock regression, corruption, immutable SQL guards,
audit rollback, 2/4/8 independent processes and kills after projection insertion,
before commit and after commit. A passing fixture is storage-boundary evidence,
not a trusted artifact resolver or real backtest result.

Migration refuses pre-existing development rejections rather than silently
forgetting their history. Such a database needs an explicit verified projection
migration before upgrade; no backfill is fabricated here. Existing migrations
and A-share archives remain unchanged. Migration 7 is not deployed to production.

Disable submissions/acquisition/completion through the existing default-deny
admission policy before rollback; retain committed jobs, projections, receipts
and audit. Do not drop migration 7 or delete its SQLx record. Older binaries
without this migration refuse the newer schema, so a binary-only downgrade is
not supported after deployment. Recovery requires a schema-aware compatibility
build or a forward fix while writers stay disabled. Never restore an old
database snapshot over newly committed research history. Before deployment,
reverting this commit is sufficient because the production schema is untouched.
