# ADR 0049: Bounded discovery lifecycle and recovery

Status: accepted for implementation; Phase 10 unit 4 remains in progress until
the task's behavioral gates, commit, push and remote verification pass.

## Requirement

An authenticated owner can pause, cancel and resume a bounded discovery job
without replacing its frozen inputs, resetting its budget or resending an
uncertain paid invocation. Control must work across OS processes and survive
restart. Infrastructure failure is never quantitative factor rejection.

## Decision

Append Discovery-only `Paused=9` to the job contract. Paused jobs have no active
lease or outcome; their original attempt, history and reservations remain. Do
not reuse Queued, because the outer scheduler must not restart paused work.
Reuse jobs, model_steps, tool_results, command receipts and transactional audit.
Do not add a run table, workflow service or duplicate conversation store.

Pause, Cancel and Expire are narrow revision-CAS commands. Their authorization
requires a current authenticated Discovery owner, the original run scope and a
valid persisted job. They only reduce execution authority and do not require
live plan files, market files or an enabled model executor. The stop-only
deployment path disables the discovery configuration while retaining identities.
Job corruption and caller impersonation still deny. Responses expose job handles,
not prompts, leases or raw errors. Expire only terminalizes jobs whose original
absolute deadline has elapsed. Pause after that deadline also expires the job.

Resume requires the original supported plan, development data and identity. It
atomically claims a fresh lease from Paused and continues the same ordinal. A
replayed Resume returns evidence and never borrows another handler's execution
right. Queued and expired-running execution retain their existing entry point.
The original submitted_at + maximum_wall_time (at most 120 seconds) never moves.
The runtime checks durable state while waiting on external work; a control CAS
fences further commits and cancels local waits. A committed dispatch may already
have reached the Provider and may be billed; cancellation cannot undo it.

Paid dispatch occurs at most once per ordinal. After Dispatched or Ambiguous,
including a Provider ABSENT response, only LookupInvocation is allowed. Safe
active lookup and uncommitted read-only tool verification each permit at most
three attempts per ordinal. Before each attempt, persist its counter and a
250 ms retry-not-before timestamp. Restart and Pause/Resume do not reset these
fields. Permission, validation, conflict and corrupt-evidence errors are not
automatically retried. Exhaustion produces a static infrastructure outcome and
retains reservations. Retry waits and operations are bounded by caller, lease
and original job deadlines; clock regression fails closed.

Explicit Reconcile is evidence-only for Paused or terminal jobs. One bounded
authenticated lookup may append a matching Provider response under revision
CAS, without acquiring an execution lease, invoking tools, producing a candidate,
changing a terminal outcome or refunding budget. Its fresh request deadline is
independent of the elapsed execution budget. It does not reset active retry
counters. Repeated manual requests remain individually authenticated, bounded
and audited when evidence changes. Reconcile requires the original supported
plan and connector binding, but no market-data reads. Missing plan evidence
permits stop-only control, not an arbitrary replacement Provider connection.

The API enforces expiry before advancing execution and offers explicit Expire.
No unattended background deadline sweep is claimed by this delivery unit;
the persistent outer scheduler belongs to Phase 11. A paused job can therefore
remain recorded as Paused after its deadline until a lifecycle command confirms
expiry, but it cannot resume or authorize paid work after that deadline.

## Compatibility and rollback

Migration 0013 adds bounded retry metadata, the Discovery-only Paused constraint
and a new writer fence. Stop old writers before applying it; retain all original
requests, responses, ordinals, budgets and audit bytes. Support the specifically
reviewed previous descriptor alongside the current descriptor; never accept an
arbitrary unknown schema or rewrite frozen plan/job bytes for compatibility.

Rollback disables new discovery execution on a migration-aware binary. Keep
authenticated stop operations available and preserve every history and ambiguous
reservation. Do not drop tables, remove receipts or apply destructive down
migrations. Old binaries may refuse the newer schema and are not write-compatible.

## Acceptance

Require real PostgreSQL and mTLS/Provider v1/v2 pause/resume/cancel workflows,
no-discoverer stop control, denied cross-run/actor operations, old-profile
compatibility, bounded retry and late-response reconciliation. Require independent
2/4/8-process contention and kill/restart before and after each new storage
transition; verify stale-worker denial, immutable requests and conservative spend.
Exercise expiry, commit-time clock regression, corruption and rollback of failed
transactions. Preserve three-language job vectors, original numerical goldens,
Rust format/Clippy and the workspace gates. Synthetic suppliers do not establish
live model verification or research admission.
