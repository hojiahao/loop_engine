# Holdout approval record canonicalization v1

This server-owned format defines `HoldoutApprovalRecord.approval_record_sha256`.
It is independent of Protobuf encoding and SQL row representation. An approval
ID is an opaque server-assigned identifier, not a content-addressed period ID.
Consumers receive the typed record and its digest; no new client-side hashing
implementation or remote approval endpoint is advertised by this checkpoint.

Canonical bytes are UTF-8 JSON without a BOM, whitespace outside strings, or a
trailing newline. Object keys are emitted in exactly the order listed below.
Strings escape quote and backslash, use the short JSON escapes for backspace,
tab, newline, form feed and carriage return, and lowercase `\u00xx` for other
U+0000 through U+001F characters. Other Unicode is emitted directly, without
normalization. Lone surrogates are not valid UTF-8 strings. Slash, U+2028 and
U+2029 are not escaped. Domain numbers are normalized decimal strings, never
JSON number tokens. The two optional artifact fields below use JSON `null`
when absent; this is specific to this format, not the period/plan format.

Top-level field order:

```text
schema                        = "loop.holdout-approval-record/v1"
holdout_approval_record_id     = opaque, validated ASCII identifier
holdout_period_id             = strict SHA-256 period identity
freeze_manifest_sha256        = strict SHA-256 text
approved_by                   = actor object below
reason                        = exact human attestation text
evidence                      = ordered array of full artifact projections below
approved_at                   = UTC RFC3339 with nine fractional digits
expires_at                    = UTC RFC3339 with nine fractional digits
holdout_evaluation_plan_id     = strict domain-separated SHA-256 plan identity
evaluation_plan_sha256        = strict raw plan SHA-256 text
evaluation_plan_entry_count   = normalized decimal string, 1..4096
canonical_period_sha256       = equal to holdout_period_id
```

The actor has exactly `actor_id`, `kind`, `display_name`, and
`authenticated_subject`, in that order. Kind is always `human`. Actor identity
and metadata come from the separately authenticated principal, not from a
trusted-looking label in the request. IDs use the repository's bounded ASCII
identifier grammar. Subject is nonempty and at most 4,096 UTF-8 bytes; display
name is at most 4,096 bytes. Both reject control characters.

Reason is non-blank, at most 4,096 UTF-8 bytes, and rejects control characters
except newline and tab. Evidence has at most 64 entries with distinct artifact
IDs. Order is preserved and affects identity and retry semantics. Each evidence
object contains exactly:

```text
artifact_id, uri, sha256, schema_name, schema_version, schema_sha256,
media_type, byte_size, row_count, created_at, manifest_sha256
```

Artifact identity, locator and schema use the existing `ArtifactRef` validator.
`schema_version`, `byte_size`, and present `row_count` are decimal strings.
`row_count` and `manifest_sha256` are `null` when absent. Digests use `sha256:`
plus 64 lowercase hex digits. `created_at` is UTC with exactly nine fractional
digits, is not before the Unix epoch, and cannot be after approval. Approval and
expiry are whole-millisecond
server instants in the supported non-negative timestamp range; expiry must be
strictly later and at most seven days after approval. The canonical document is
at most 256 KiB.

```text
approval_record_sha256 = SHA-256(
  ASCII("loop.holdout-approval-record/v1") || 0x00 || canonical_record_bytes
)
```

The record's own `approval_record_sha256` field is omitted from canonical bytes.
Changing any actor, reason, evidence, timestamp, or freeze/plan binding changes
the digest. A read reconstructs this document from validated fields and requires
exact equality with its stored canonical document and digest. Merely recomputing
a Protobuf blob checksum does not make a modified attestation valid. Database
administrators can still rewrite data and all hashes; external immutable audit
anchors are a later deployment requirement, not a claim of this local hash.
