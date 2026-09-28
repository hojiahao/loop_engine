# ADR 0008: Immutable human holdout approvals

- Status: Accepted for the Phase 3 storage boundary
- Date: 2026-09-08
- Owner: hojiahao

## Decision

`loopd` persists individual human attestations before grant issuance. Its
`HoldoutRepository::record_approval` accepts the existing narrow Protobuf command
and a separately authenticated principal. Caller metadata must exactly match
that principal; service, agent, scheduler, and unspecified kinds cannot approve.
The Rust type is not itself authentication. Production transport interception
and protected reference resolution remain separate gates, so this checkpoint
does not expose a mutating RPC or issue a data capability.

The existing SQLx framework manages forward-only PostgreSQL migrations. Add
`0003_holdout_approvals.sql`; never rewrite the already deployed `0001` and
`0002` or replace the database. Explicit SQL defines immutable rows, foreign
keys, validity bounds, and indexes. SQLx transactions bind the approval insert,
canonical audit append, immutable command receipt, and clock watermark update.
Administrative migrations remain separate from runtime DDL-free verification.

The server-owned holdout policy must verify the exact freeze, canonical period,
complete canonical evaluation plan, referenced backtest contents, pinned
approval policy, and evidence in a bounded pre-resolved registry. Its default
implementation denies. Test registries use synthetic references and are not
production freeze/BacktestSpec implementations. No network I/O belongs inside
the ledger write lock. The repository independently validates wire identities,
bounds, human identity, exact persisted period, and server time. New approvals
require a sealed period. Validity is half-open, positive, and at most seven days;
evidence cannot have a creation timestamp after approval.

## Identity and replay

The server assigns an opaque approval ID and the approval time. The canonical
record format in `docs/specs/holdout-approval-canonicalization-v1.md` binds every
attestation field except its self-digest. Protobuf blob checksums detect storage
corruption but are not approval identities. Reads and retries verify both raw
blob checksums and canonical record identity, all indexed projections, and the
period binding.

Receipts are scoped to authenticated actor ID, operation, and idempotency key.
Transport request ID and requested time are excluded from semantic retry
identity; actor subject, reason, evidence order, correlation, causation, and all
freeze/plan bindings are not. An identical retry returns the original approval,
including its original expiry, and creates no new event. It can return an
expired historical record but cannot renew it. Current authority and trusted
references must still resolve. Changed semantic content conflicts.

Cancellation or failure before commit rolls back the entire command. If commit
outcome is uncertain, retry the same key. PostgreSQL ledger locking supplies
cross-process serialization; clock regression fails closed. Approval lookup
requires authorization before querying the ID and again for its resolved period.
Neither an approval ID nor a successful read is a capability.

## Remaining work

Grant issuance must independently validate current, distinct authenticated human
approvals against the frozen policy; different actor aliases must not count as
different humans. Grant attachment will be a separate immutable relation so
each approval can be attached at most once without rewriting this record.
Single-grant period transitions, grant expiry/revocation, atomic plan-derived
batch consumption, and production authorization are not implemented by this
approval checkpoint. Phase 3 remains open until its complete exit gate passes.
