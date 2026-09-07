# Protocol selection canonicalization v1

## Purpose

`ProtocolSelectionSnapshot.selection_sha256` is an integrity claim over the
negotiated protocol contract. It is never a hash of Protobuf serialization.
Every producer and consumer rebuilds the closed document below and compares the
digest in constant time before persistence, dispatch, lease, or resume.

## Identity

```text
selection_sha256 = SHA-256(
  ASCII("loop.protocol-selection/v1") || 0x00 || canonical_selection_bytes
)
```

The wire field is the raw 32-byte digest. Textual digests inside canonical JSON
are `sha256:` followed by 64 lowercase hexadecimal characters.

## Canonical document

The encoding is UTF-8 (all admitted values are ASCII), without BOM, whitespace,
or a trailing newline. Objects are closed and emit fields in this exact order:

```text
schema, selected_package, enabled_features, effective_limits,
server_build_version, server_build_sha256, schema_descriptor_sha256,
selected_at, client_build_version, client_build_sha256
```

`schema` is exactly `loop.protocol-selection/v1`. `selection_sha256` itself is
excluded. `enabled_features` is sorted, unique, and uses the protocol feature
grammar. `effective_limits` emits, in order:

```text
maximum_unary_bytes, maximum_stream_event_bytes,
maximum_canonical_ast_bytes, maximum_ast_nodes, maximum_ast_depth,
maximum_page_records, maximum_identity_bytes, maximum_artifact_uri_bytes
```

All limit values are JSON strings containing normalized unsigned decimal
integers. `selected_at` is the closed object `seconds,nanos`; both values are
decimal strings, seconds may be negative, and nanos is in `[0, 1000000000)`.
Build versions and package/features satisfy the bounded grammars enforced by
the protocol validator, so no JSON escaping is required or permitted. A
package is one or more lowercase identifier segments followed by `.vN`, where
`N` is a positive decimal integer with no leading zeroes; `loop.v0` and
`loop.v01` are invalid.

## Validation boundary

The selection time must not be later than the enclosing job's `submitted_at`.
The negotiated limits must be non-zero, within the v1 hard maxima, and obey the
cross-limit constraints. A worker also verifies that both pinned build
identities and the descriptor are locally available; availability is a runtime
compatibility check, not part of canonicalization.

Implementations expose symmetric producer functions for canonical bytes and
the digest. Validation always recomputes both from the DTO; it never trusts a
caller-supplied digest or a generic JSON serializer.
