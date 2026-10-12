# ADR 0046: Read-only recovery of Provider invocation receipts

Status: implementation and local quality/build, complete regression and actual
container gates passed; delivery on `feature/run-harness`, with exact-commit
remote CI required before closing the unit. Phase 10 delivery unit 1.

## Requirement

A persisted model step may outlive a Provider process or the five-minute command
freshness window. Replaying the original InvokeModel request then fails identity
validation; updating its timestamp changes the request digest. A new key could
charge again. Recovery therefore needs a fresh authenticated query for the old
receipt before the Rust Harness can safely own durable model steps.

## Decision

Add `ProviderService.LookupInvocation` to the existing isolated Provider service.
Use the same mTLS certificate-to-principal mapping, actor validation, fresh
CommandContext, mandatory RPC deadline and concurrency bound as invocation.
Only query the authenticated actor's journal namespace. No caller-supplied actor
can widen that authority; no model, research or holdout access is acquired.

The query supplies the original request ID, idempotency key and SHA-256 of the
normalized InvokeModel envelope. It carries no prompt. Read the existing private
claim/result files; do not create a claim, repair storage, load a supplier secret,
consume a generation allowance or contact any upstream service. Recovery works
when the original route or model pin has been removed from current configuration.

Expose `providerd --recover-only` for an actual process restart when the generation
catalog is missing, expired or no longer matches deployment configuration. This
mode validates the existing journal without creating it, skips catalog activation,
does not instantiate model plugins, and denies InvokeModel/StreamModel. It still
requires a valid deployment, TLS material and actor ACL. Normal generation startup
keeps its catalog checks; operators do not bypass them to recover a receipt.

Return three explicit observations: ABSENT, AMBIGUOUS or COMPLETED. Missing
storage is an outage, not ABSENT. A partial claim is AMBIGUOUS because publication
precedes writing its full bytes. A complete claim without a result preserves the
recorded reservation. A completed result must match its checksum, decode as a
valid response envelope and match the original request ID. Conflicting identity
and corrupt evidence are distinct typed errors; neither permits regeneration.

The returned USD reservation is the original conservative ceiling, not an
invoice. An incomplete claim may not disclose a trustworthy reservation; the
caller keeps its own prior ceiling. ABSENT is only a point-in-time observation:
another request may still be approaching claim publication. No lookup outcome
automatically releases a budget or authorizes sending a new invocation.

Reads have bounded record sizes, no symlink following, private ownership/modes,
nonblocking special-file opens, reads anchored to the open directory descriptor,
directory identity revalidation, and a five-second maximum service deadline
(or the smaller deployment limit). A cancelled OS read keeps its concurrency
slot until it ends. This endpoint has no writers to leave running after cancel.
Provider storage still relies on its existing trusted administrative parent;
checksums detect damage, not compromise of the service's OS identity.

## Compatibility and scope

The additive RPC and `provider.invocation-lookup.v1` capability identity do not change existing
claims/results or the baseline descriptor. Rust, Python and TypeScript bindings
consume shared wire fixtures. This unit checks method availability; it does not
implement metadata feature negotiation. Old servers return UNIMPLEMENTED; callers must
stop recovery, never fall back to a paid invocation. Existing model and stream
methods retain their validation and journal behavior.

This is a callable recovery workflow, not a complete durable Harness. Persisted
Rust reservations, dispatch, lease takeover, context/tool execution and run
lifecycle are the subsequent Phase 10 units. Historical recovery does not make
a removed model available or grant factor admission.

## Acceptance and rollback

- Actual mTLS/gRPC and compiled CLI: original response recovered with zero extra
  supplier calls, after restart, with old timestamp, missing credentials, removed
  route and unavailable generation catalog. Missing journal is never recreated
  by the recovery-only startup path.
- Exact digest/request identity, actor isolation, certificate rejection,
  metadata denial, freshness, deadlines, cancellation and concurrency limits.
- Partial/missing claim, missing result, orphan result, bad checksum, invalid
  response, unsafe files/directories and bounded records all fail conservatively.
- Shared three-language fixtures, additive descriptor checks, full Provider
  regression and workspace quality gates; task commit/push and remote CI.

Rollback by disabling callers of the new RPC and restoring the previous Provider
binary. Preserve all journal state and Rust-side reservations. No schema migration,
deletion, down-migration or rewritten research/audit history is needed.
