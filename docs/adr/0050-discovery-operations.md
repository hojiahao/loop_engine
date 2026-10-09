# ADR 0050: Usable Discovery operations and deployment recovery

Status: implemented and verified. Phase 10 unit 5 is published as `b692e30`;
correction `5a25008` passes CI `37733309351` (7/7). Installed CLI workflows,
negative paths and production control-service recovery are accepted.

## Requirement and decision

Make the existing bounded Discovery Harness operable through `loopctl discovery`.
Reuse its mTLS RPCs, PostgreSQL receipts and immutable plans. The CLI receives no
database credentials, Provider API keys, holdout capabilities or raw datasets.
Do not add a workflow service or duplicate persistent state.

Use a private, strict `loop.client/v1` JSON configuration with an HTTPS endpoint,
server name, absolute CA/certificate/key paths and the registered Discovery actor.
Validate regular files, ownership, bounded size, private key/config modes and
no symlinks. Never print file contents, server error messages or transport details
that may contain credentials. No HTTP/plaintext or TLS-verification bypass.

Commands are start, execute, status, pause, cancel, resume, expire, reconcile and
events. Start reads a bounded canonical DiscoveryJobInput protobuf file prepared
from an administrator-pinned plan. Mutations require an explicit idempotency key;
job mutations require an explicit expected revision. Fresh request ID/time do
not change stable correlation/causation identity derived from that command key.
Never auto-refresh CAS, retry mutations, create a replacement job, or resend a
paid invocation. A lost command can be retried with its original key and inputs.
Execute retains the service's existing lease/takeover semantics.

Emit a versioned machine-readable JSON envelope by default, exact money and
integer identities, safe status/step names and a canonical candidate only when
returned by validated execution. Transport success is distinct from terminal
job failure; define stable exit categories. Bound connection and RPC timeouts.
SIGINT drops the client RPC and exits distinctly; it cannot claim to cancel the
durable job. Durable cancellation is the explicit cancel command.

Read-only status must remain available in a stop-only deployment. Read job and
model history in one authorized transaction using the original current owner
and run scope. A pure projection exposes only handle, step state and conservative
reservations. Add `plan_verified` to the narrow view: false for stop-only metadata,
true only after resolving the frozen plan and history; never return a candidate
when false. Corrupt persistent evidence still fails closed. Execution and normal
candidate reads retain strict plan validation.

Add a narrow job-scoped ListDiscoveryEvents RPC. It returns only sequence,
timestamp and an allowlisted typed operation, never raw audit payloads, prompts,
credentials, other job targets or holdout types. Authenticate ownership before
querying. A nonzero cursor must belong to that job; page size is 1..100. A bounded
indexed query uses (job_id, sequence), verifies event checksum/target and its
immediate ledger predecessor, and projects only supported lifecycle events.
This filtered view is not represented as proof of the complete global audit
chain. Add only the required index; no event table or rewritten history.

## Deployment and acceptance

Document binary installation, existing plan/input preparation, private client
configuration, explicit migration credentials, endpoint start, status/events,
restart/resume and disabling generation while retaining stop/status/events.
Service termination must handle SIGTERM as well as SIGINT. Tests run actual
loopctl child processes against authenticated loopd and the actual Provider with
a local synthetic supplier and PostgreSQL. Include wrong certificates/actors,
stale revision, key conflicts, deadlines, signal cancellation, JSON/exit behavior,
restart, stop-only observation and no duplicate supplier calls.

Pin specifically reviewed preceding descriptors when the additive observation
protocol changes. Preserve frozen job bytes; unknown descriptors still deny.
Remote CI of unit 4 is tracked concurrently at the user's explicit request;
unit 5 cannot be declared complete before both tasks' remote gates pass.

Rollback disables CLI writes or discovery execution on a migration-aware binary.
Retain plans, journals, receipts, budgets and audit. The observation index is
additive and may remain; no destructive down-migration. This unit is operational
acceptance of the bounded Harness, not the autonomous outer Loop or formal
research admission. Paid-data and model-credential gates remain in force.
