# Discovery lifecycle

The authenticated Discovery gRPC service controls the existing bounded v1/v2
research plan. It does not admit a factor or implement the outer research Loop.
The operational CLI is documented in [Discovery operations](discovery-cli.md).

| RPC | Required state | Effect |
| --- | --- | --- |
| `PauseDiscovery` | Queued/Leased/Running/Paused | Paused, lease removed, evidence retained |
| `CancelDiscovery` | Queued/Leased/Running/Paused | Cancelled, no subsequent execution |
| `ResumeDiscovery` | Paused | New fenced lease, same plan/ordinal/budget |
| `ExpireDiscovery` | Nonterminal and original deadline elapsed | BudgetExhausted |
| `ReconcileDiscovery` | Paused/Cancelled/BudgetExhausted/InfrastructureFailed | Lookup-only evidence append |

Every request supplies CommandContext, job_id and expected_revision. Transport
mTLS determines identity; actor metadata cannot impersonate another owner. Use
a fresh request ID/time and preserve the original idempotency key and semantic
command on retry. Caller timeout is required, positive and at most 120 seconds.
Successful receipt replays return current evidence without granting execution.

Paused is distinct from Queued. `ExecuteDiscovery` never silently unpauses work.
Resume does not reset the original absolute deadline, recorded attempts, request
identity, safe-retry counters or cumulative token/USD reservations. A late Resume
atomically records expiry using the same caller CAS and receipt. Cancelled and
failed runs cannot be resumed; creating another job is a new separately budgeted
research attempt, not recovery of this one.

Pause/Cancel/Expire need the authenticated original Discovery owner and run scope
but do not read live research or plan files. If a plan cannot load, remove the
optional discovery executor configuration and retain the owner's identity
configuration to start a stop-only service. This does not grant model or data
execution. Corrupt persisted job identity still denies control.

Active operations poll durable authority every 250 ms with a one-second read
ceiling. Database failure stops local work. Control fences later writes and
dispatch authorizations; it cannot retract a dispatch committed before the stop.
The Provider may therefore bill a cancelled request. Its reservation remains.

There is no automatic paid resend, including when Provider lookup returns ABSENT.
Each ordinal permits at most three active lookup attempts and three uncommitted
read-only tool verification attempts. The count is committed before each attempt;
a crash after that commit consumes it conservatively. Attempts have a persisted
250 ms backoff and remain bounded by the current lease and original deadline.
Permission, malformed evidence and request conflicts are not transient errors.
Exhaustion records InfrastructureFailed, retaining all invocation reservations.

Explicit Reconcile performs one fresh, bounded lookup (at most five seconds).
It requires the original supported plan and connector identity, but does not
read market data or execute a tool. A late response can complete step evidence
while the job remains Cancelled or BudgetExhausted. It cannot create a candidate
or refund uncertain spend. If the plan/connector evidence is unavailable, only
stop control remains available. Manual Reconcile does not reset the active
retry budget and does not automatically retry itself.
An existing Reconcile receipt remains replayable after a later Resume or model
ordinal advances. Replay returns the current status without another lookup or
candidate; it verifies the original command and both receipt checksums first.

Expiry is enforced whenever execution advances and can be recorded by Expire.
No unattended background sweeper is provided here: a paused/crashed job may keep
its recorded state after the deadline until a lifecycle command confirms expiry,
but cannot obtain fresh execution authority beyond that deadline.

Migration 0013 requires stopping old writers before migration. It preserves
requests, responses, ordinals, budgets, receipts and audit, adds bounded retry
fields and fences earlier writers. The reviewed previous context descriptor and
current descriptor are supported for frozen v1/v2 plans; unknown schemas deny.
Rollback disables discovery execution on a migration-aware binary while keeping
stop control and history. Do not drop tables or reclaim ambiguous reservations.
