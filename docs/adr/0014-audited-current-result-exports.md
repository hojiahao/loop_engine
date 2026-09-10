# ADR 0014: Audited current-result metadata exports

- Status: Accepted for implementation; remote acceptance pending
- Date: 2026-09-09
- Owner: hojiahao

## Decision

Add an internal `BacktestRepository::export_current` command. It shares the
ordinary current-result gate, requires both read and export permissions and
retains the separate holdout read policy. The transport principal is independent
of caller metadata. Every attempt, including receipt replay, resolves current
permissions, immutable evidence and all six current-context fingerprints.
An old successful receipt never bypasses revocation, stale inputs or missing
reference availability. Contexts must be explicit, not moving `latest` aliases.

Reuse `command_receipts` under a distinct operation namespace. An immutable
receipt binds the source terminal job, exact result metadata, current context,
original request ID and acceptance time. Protobuf bytes are checksummed for
storage integrity only, never used as semantic research identities. Request ID,
request time and deadline are transport retry metadata; other changed content
under the same principal/operation/key conflicts. Export leaves the source job
and metrics unchanged.

The receipt, `CommandAccepted` audit append and monotonic storage clock update
commit in one transaction. That event records metadata-release acceptance, not
external file delivery. A rollback or cancellation before commit publishes no
metadata; an uncertain commit can be retried without a second acceptance event.
An explicit request deadline is limited to 30 seconds, checked after the bounded
write lock and again before commit. The whole asynchronous command also has a
30-second timeout; connection and SQL limits remain in force. A timeout during
commit can have an unknown outcome, resolved only through an authorized retry.

This is a metadata-release boundary, not a CSV writer, capability mint, dataset
download or external destination API. Returned artifact references remain
subject to independent storage permissions. Receipt values cannot authorize
future access. Production policies continue to deny until real authentication
and immutable reference registries are wired; this does not close Phase 4.

## Verification

Test immutable replay across restart, every stale component, missing contexts,
revoked permissions, actor spoofing, changed retry inputs, deadlines, clock
regression, corrupt receipts/results and audit rollback. Exercise 2/4/8
independent OS writers for same-key retries and distinct exports. Kill writers
after receipt insertion, before commit and after commit, then verify one
receipt/audit pair, unchanged source metrics and authorized recovery.

## Scope And Rollback

This change addresses review finding 2: a consumer must not export historical
metrics as current after their inputs change. It does not add a database table,
service, artifact store or generic export framework. Reusing the read gate and
existing receipt/audit transaction avoids a second definition of freshness.

Before any future production exposure, transport handlers must call this command
and retain independent artifact authorization. Disable that handler or restore
the previous binary to roll back. The database schema is unchanged; old binaries
do not interpret the new operation's receipts, while its existing generic
`CommandAccepted` events remain readable. Keep all committed receipts and audit
events. Do not delete history or run a destructive down-migration. The currently
undeployed migration 6 belongs to ADR 0013, not this export change.
