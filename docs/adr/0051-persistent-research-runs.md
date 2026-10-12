# ADR 0051: Persistent research runs and cross-job budgets

Status: accepted for implementation; Phase 11 unit 1. Completion requires the
actual CLI/Provider/PostgreSQL workflow, negative and process/crash gates,
documentation, a Chinese task commit, push and remote CI.

## Requirement and scope

A human operator starts an administrator-pinned, finite research run. Its
discovery jobs share durable round, step, input-token, output-token, USD and
absolute-time ceilings. Restart, new child IDs and uncertain model calls cannot
replenish those ceilings. This task operates bounded Discovery jobs and records
their completion; numerical evaluation, feedback, semantic checking, factor
admission and unattended scheduling are subsequent Phase 11 delivery units.

## Ownership and protocol

Add a narrow RunService with start, step and status commands, exposed through
`loopctl run`. Start selects an existing immutable plan reference. Step requires
the run ID, expected revision and an explicit idempotency key. No RPC accepts a
replacement model, child ID, executor actor, prompt or budget. The response is a
bounded run projection and current child handle, never raw prompts, datasets,
holdout authority or an admitted factor.

The mTLS-registered Human/Operator owns the run. An independently registered
Discovery/Agent identity from the existing server deployment executes its
children. Do not turn a caller-supplied actor into authentication or fabricate
an Agent principal from the human caller. The server resolves a scoped internal
execution proof after checking the owner, current revision, child and frozen
plan. Parent receipts/audit identify the human; child/model receipts identify
the fixed executor and retain parent causation. Existing Agent CLI configuration
remains compatible; the run CLI uses an explicit Human configuration profile.

The run plan fixes the run ID, owner, existing Discovery plan, maximum rounds
(1..64), cumulative ceilings and maximum elapsed time. The existing immutable
Discovery input, executor and protocol selection are copied into the durable
run specification. Plan/identity drift denies execution. Authorized status is
available without live execution plans, with `plan_verified=false`.

## Atomic state and accounting

Use one PostgreSQL `research_runs` aggregate; reuse jobs, model steps, command
receipts and audit. Do not add another queue, workflow service, conversation
store or duplicate numerical results. Start rejects a run ID already present
in historical jobs, rather than silently adopting unbudgeted work.

Reserve a complete child JobBudget before creating it. Parent reservation,
child insertion, immutable receipts and audit append share one transaction.
Reuse a store-private transaction submission helper; never nest a second public
submission transaction. At most one child is current. The common job insertion
boundary denies unbudgeted jobs for managed run IDs; only the run transaction
has the private insertion authority. Concurrent legacy starts and run starts
share the existing ledger serialization. A migration writer fence protects
managed-run inserts from older writers, not from database administrators.

Reservations are conservative ceilings, not invoices. Initially no reservation
is refunded, even for unused or ambiguous calls. Child model reservations consume
that child's allowance and are not charged twice to the parent. Each next child
must fit the remaining step/token/USD allowance and its original wall-time
ceiling must fit before the parent deadline. Do not clip a frozen child budget.
Exhaustion or terminal failure never creates a replacement paid job.

Start creates the first real child; insufficient initial allowance denies the
whole command. The first child's ID anchors parent command receipts because the
existing receipt schema requires a real job foreign key. Never store a RunId as
a JobId or create a fake placeholder. Step executes the current child through
the existing Harness, then atomically accounts for terminal completion and
creates the next child if allowed. Run revision serializes advancement; the
existing child lease serializes model execution. A crash after child completion
but before advancement reuses committed child evidence, without model dispatch.
Repeated step receipts are observational and never authorize another round.

Run state becomes completed at the frozen round limit, budget-exhausted when
another whole child cannot fit, or infrastructure-failed/deadline-exceeded as
appropriate. A child failure is not empirical factor rejection. Terminal runs
cannot create children. Stop/status does not invent successful research results.

## Compatibility, acceptance and rollback

Migration 0015 is additive and preserves all existing research/audit records.
Stop previous writers before deployment. Updated runtime verifies exact schema
checksums; do not deploy an older binary over this schema or run destructive
down-migrations. Disable new run execution while preserving owner observation,
child stop controls, reservations, immutable plans and receipts.

Exercise installed CLI plus actual PostgreSQL, Provider transport and a local
synthetic supplier: two rounds, deterministic stop, restart and no duplicate
paid dispatch. Test owner/role/run spoofing, missing or changed plans, replay
and CAS conflicts, exhaustion, clock regression, expiry, corrupt evidence and
both legacy submission bypasses. Require 2/4/8 independent OS writers and
kill/restart before and after atomic reservation/advancement commits. Preserve
existing Agent CLI, numerical goldens and three-language protocol contracts.
Fixtures use no paid credentials or production research writes. Deployment of
the new schema is separate from local/CI acceptance of this task.
