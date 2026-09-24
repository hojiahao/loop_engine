# Audit event canonicalization v1

## Purpose

This specification defines the bytes committed by `AuditPayload.payload_sha256`
and `AuditEvent.event_sha256`. It makes an audit ledger verifiable across Rust,
TypeScript, Python, Protobuf runtime upgrades, and additive wire-schema changes.

Protobuf binary output is not canonical. Deterministic Protobuf serialization,
Protobuf JSON, generated-language objects, database row encoding, and log text
are therefore forbidden hash inputs.

## Digest representation

`loop.v1.Sha256Digest` carries exactly 32 raw digest bytes on the wire. This
document renders those bytes as `sha256:` followed by 64 lowercase hexadecimal
characters. A producer rejects wrong-length bytes, uppercase hexadecimal,
abbreviated values, and values tagged with another algorithm.

The hash domain prefixes and NUL separators below are literal bytes:

```text
payload_sha256 = SHA-256(
  ASCII("loop.audit-payload/v1") || 0x00 ||
  ASCII(schema_name) || 0x00 ||
  ASCII(canonical_schema_version) || 0x00 ||
  canonical_payload_bytes
)

event_sha256 = SHA-256(
  ASCII("loop.audit-event/v1") || 0x00 || canonical_event_bytes
)
```

Neither hash input has a BOM, trailing newline, nor trailing NUL.

## Canonical payload

`schema_name` is a dot-qualified lowercase ASCII identifier, and
`schema_version` is a positive `uint32` rendered without a sign or leading
zeroes. Their values select one immutable payload schema from the audit schema
registry. The selected schema defines exact fields, field order, variants,
limits, and semantic validation. Unknown schemas and versions fail closed.

`canonical_payload_bytes` is the registered schema's dedicated canonical JSON,
not arbitrary JSON with its keys sorted. Every v1 payload schema must:

1. use UTF-8 without BOM and emit no insignificant whitespace;
2. define a closed object/variant shape and exact field order;
3. reject duplicate, unknown, out-of-order, and missing required fields;
4. represent integers and exact decimals as normalized decimal strings, never
   JSON number tokens;
5. forbid binary floating-point, `NaN`, infinities, and implicit `null`;
6. define ordering and maximum counts for every array; and
7. limit canonical bytes to 256 KiB or a smaller schema-specific limit.

The server parses and validates the payload, writes it again with the selected
dedicated writer, requires byte equality with the submitted bytes, and then
recalculates `payload_sha256`. A mismatch is a validation/integrity failure and
causes no append.

### Closed action registry

An action is not a free label attached to arbitrary canonical JSON. Each v1
action selects exactly one payload schema and a closed set of target variants.
The server validates this binding before calculating or accepting an event
digest. For subject-bearing payloads, the first identity field must equal the
typed event target value.

| Action | Required payload schema/version | Required target |
| --- | --- | --- |
| `command_accepted` | `loop.audit.command_accepted`/1 | run, job, factor, backtest, snapshot, or artifact |
| `state_transitioned` | `loop.audit.state_transitioned`/1 | run, job, backtest, snapshot, or holdout period |
| `factor_admitted` | `loop.audit.factor_admitted`/1 | matching factor spec |
| `factor_rejected` | `loop.audit.factor_rejected`/1 | matching factor spec |
| `override_authorized` | `loop.audit.override_authorized`/1 | matching factor spec |
| `readmission_requested` | `loop.audit.readmission_requested`/1 | matching factor spec |
| `readmission_decided` | `loop.audit.readmission_decided`/1 | matching factor spec |
| `holdout_grant_issued` | `loop.audit.holdout_grant_issued`/1 | matching holdout grant |
| `holdout_grant_consumed` | `loop.audit.holdout_grant_consumed`/1 | matching holdout grant |
| `artifact_exported` | `loop.audit.artifact_exported`/1 | matching artifact |
| `holdout_approval_recorded` | `loop.audit.holdout_approval_recorded`/1 | matching approval record |

The immutable v1 payload objects use the following exact field order. Scalar
fields are JSON strings; `approval_records` is the one closed object array
defined below. Identity fields use their typed ASCII or full
`sha256:<lowercase-hex>` encoding; timestamps use the event timestamp profile;
reason and summary text are non-empty and at most 4,096 UTF-8 bytes.

```text
command_accepted:
  command, request_id, summary
state_transitioned:
  from, to, reason
factor_admitted:
  factor_spec_id, decision, evidence_artifact_id
factor_rejected:
  factor_spec_id, rejection_code, reason, evidence_artifact_id
override_authorized:
  factor_spec_id, override_kind, authorized_by_actor_id, reason,
  approval_reference, evidence_artifact_id
readmission_requested:
  factor_spec_id, original_rejection_event_id, requested_by_actor_id, reason
readmission_decided:
  factor_spec_id, original_rejection_event_id, disposition,
  decided_by_actor_id, reason
holdout_grant_issued:
  holdout_grant_id, holdout_period_id, freeze_manifest_sha256,
  holdout_evaluation_plan_id, approval_records, capability_class,
  authorization_decision
holdout_grant_consumed:
  holdout_grant_id, holdout_period_id, holdout_evaluation_plan_id, job_batch_id,
  capability_class, authorization_decision
artifact_exported:
  artifact_id, export_class, policy_id, destination_class
holdout_approval_recorded:
  holdout_approval_record_id, holdout_period_id, freeze_manifest_sha256,
  approved_by_actor_id, expires_at
```

Closed enum values are:

- `decision`: `admitted`;
- `rejection_code`: `duplicate`, `previously_failed`,
  `insufficient_coverage`, `deterministic_filter`, `performance`,
  `correlation`, `semantic_review`, or `policy`;
- `override_kind`: `force_admission`, `readmission`, or `policy_exception`;
- `disposition`: `admitted`, `rejected`, or `quarantined`;
- `capability_class`: `holdout_evaluation`;
- `authorization_decision`: `authorized`;
- `export_class`: `research_report`, `audit_bundle`, `data_snapshot`, or
  `factor_values`; and
- `destination_class`: `local_managed`, `approved_object_store`, or
  `user_download`.

`holdout_grant_issued.approval_records` contains 1 through 8 objects in strictly
increasing `approved_by_actor_id` ASCII order. Each object has exactly
`holdout_approval_record_id`, `approval_record_sha256`, and
`approved_by_actor_id`, in that order. Record IDs, record digests, and actor IDs
are each unique within the array. This commits the grant event to every approval
used by the authorization decision, while `capability_class` identifies only the
non-secret capability class.

Evidence is referenced by immutable artifact identity; payloads never inline
evidence, authorization capability values, destinations, credentials, or
free-form JSON.
Changing an action, payload schema, target kind, or subject identity without
changing the others is invalid even if an attacker recomputes both digests.

`factor_spec_id`, `artifact_id`, `evidence_artifact_id`, `policy_id`,
`holdout_period_id`, `freeze_manifest_sha256`, `approval_record_sha256`, and
`holdout_evaluation_plan_id` use the full SHA-256 textual form. Actor, request,
event, approval, grant, and job-batch identifiers use the canonical ASCII
domain-ID profile. Payload timestamps use the same exact nanosecond UTC profile
as `occurred_at`.

Event validation first verifies the registered canonical payload and its
digest, then validates the typed target, then applies the action binding, all
before emitting canonical event bytes or calculating `event_sha256`. A valid
registered payload paired with the wrong action fails with
`action_payload_mismatch`; a forbidden target kind or mismatched subject fails
with `action_target_mismatch`. Parser recursion/resource exhaustion and malformed
container depth fail as `non_canonical_payload`; native parser exceptions do not
escape the audit boundary.

## Canonical event document

The event hash commits to a schema-specific projection of the validated domain
event. It does not hash the `AuditEvent` wire message. The canonical object has
exactly these fields in this order:

```json
{"schema":"loop.audit-event/v1","audit_ledger_id":"ledger.primary","sequence":"1","previous_event_sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000000","audit_event_id":"audit.00000001","occurred_at":"2026-09-05T06:30:00.000000000Z","correlation_id":"correlation.01","causation_id":"causation.01","actor":{"actor_id":"actor.hojiahao","kind":"human","display_name":"hojiahao","authenticated_subject":"github:hojiahao"},"action":"command_accepted","target":{"kind":"job_id","value":"job.01"},"payload":{"schema_name":"loop.audit.command_accepted","schema_version":"1","payload_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111"}}
```

The example uses illustrative IDs and digests. It defines field order, not a
golden digest.

### Scalar encodings

- `schema` is exactly `loop.audit-event/v1`.
- `audit_ledger_id`, `audit_event_id`, `correlation_id`, `causation_id`, target
  value, and actor ID use their validated non-empty ASCII domain encoding and
  are at most 128 bytes.
- `sequence` is a normalized positive unsigned decimal string. JSON number
  tokens are forbidden.
- SHA-256 values use the textual form defined above.
- `occurred_at` is UTC with exactly nine fractional digits:
  `YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ`. Leap seconds and offsets are rejected. The
  value must round-trip to a valid `google.protobuf.Timestamp` without loss.
- Human text is valid Unicode scalar text with no implicit normalization.
  Canonically escaped JSON strings follow RFC 8785 string serialization:
  required quote, reverse-solidus, and control-character escapes only; other
  characters are emitted as UTF-8. Unpaired surrogates are invalid.

Actor `kind` is exactly one of `human`, `service`, `agent`, or `scheduler`.
`action` is the lowercase spelling corresponding to one nonzero `AuditAction`:
`command_accepted`, `state_transitioned`, `factor_admitted`,
`factor_rejected`, `override_authorized`, `readmission_requested`,
`readmission_decided`, `holdout_grant_issued`, `holdout_grant_consumed`,
`artifact_exported`, or `holdout_approval_recorded`.

The target object is a closed variant. `kind` is exactly one of `run_id`,
`job_id`, `factor_spec_id`, `backtest_id`, `snapshot_id`, `holdout_grant_id`,
`artifact_id`, `holdout_approval_record_id`, or `holdout_period_id`; `value` must
validate as the corresponding typed ID. Factor, artifact, and holdout-period
targets require the full lowercase `sha256:` form, never an alias or abbreviation.

The additive `holdout_period_id` wire variant (field 9) requires the negotiated
feature `audit.holdout-period.v1` before exposing period events to mixed-version
audit peers. Older readers may skip that unknown field but cannot interpret its
target, verify the event, or acknowledge it as accepted. They must fail closed;
lossless forwarders retain the original envelope. Existing target encodings and
hashes are unchanged. Local persistence uses the matching in-process canonical
writer/verifier; production audit RPCs remain unavailable until negotiation and
authorization are enforced.

The payload object commits to `schema_name`, `schema_version`, and the verified
payload digest. It intentionally does not inline `canonical_payload_bytes`:
the independently verified digest commits to those bytes transitively and
keeps the event document bounded. Changing the payload bytes, schema identity,
or version changes `payload_sha256` and therefore `event_sha256`.

`event_sha256` itself is excluded from `canonical_event_bytes`. Every other
identity-bearing domain value represented by `AuditEvent` appears in the
canonical projection above. Implementations use a dedicated writer with this
fixed order; a generic JSON serializer is not a conformance mechanism.

## Append-chain rules

Audit append is a compare-and-swap transaction scoped to one
`audit_ledger_id`:

1. Authenticate the caller and verify that `CommandContext.actor` describes
   the same principal.
2. Validate and hash the canonical payload.
3. Lock/read the durable ledger head and require the request's expected prior
   sequence and digest to match it.
4. For the first event, require prior sequence zero and exactly 32 zero bytes as
   `previous_event_sha256`; otherwise set `sequence` to prior sequence plus one
   and copy the exact prior `event_sha256`.
5. Allocate the immutable event ID and server event time, build the typed domain
   event, emit `canonical_event_bytes`, and calculate `event_sha256`.
6. Insert the event and advance the ledger head atomically. An idempotent retry
   returns the same event; it does not allocate a new sequence.

The ledger ID is inside the event hash, so an otherwise identical event cannot
be replayed into another ledger. Sequence and previous digest prevent deletion,
insertion, and reordering without detection. Fork detection compares both the
sequence and digest; sequence alone is insufficient.

## Verification

A verifier loads events in ascending sequence and independently:

- validates every typed field and canonical enum spelling;
- resolves the registered payload schema and recalculates its payload digest;
- reconstructs canonical event bytes and recalculates the event digest;
- checks the genesis digest, exact sequence increment, and previous digest; and
- rejects duplicate event IDs, sequence gaps, forks, unknown variants, and any
  non-canonical payload.

Required cross-language vectors cover a genesis event, a multi-event chain,
Unicode actor display text, every target/action variant, maximum boundaries,
payload mutation, schema/version mutation, timestamp mutation, sequence gaps,
event reordering, previous-digest tampering, and cross-ledger replay. Rust,
TypeScript, and Python must produce identical canonical bytes and digest bytes
for every accepted vector. The suite also cross-pairs every action with a wrong
registered schema, a forbidden target kind, and a mismatched subject identity;
re-signing any such invalid combination must still fail closed.

## References

- [Protocol Buffers: Proto Serialization Is Not Canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/)
- [RFC 8785: JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785)
