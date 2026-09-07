# Holdout canonicalization v1

## Purpose

This specification defines the only byte representations used to identify a
locked historical holdout period and its complete frozen evaluation plan.
Protobuf messages, database keys, display labels, and language-native object
serialization are transport or storage representations and are never identity
formats.

The period identity prevents aliases for the same locked sample. The plan
identity prevents a caller from changing factors, backtest specifications, or
budgets after approval. Both formats are closed, bounded, and independently
validated before any approval, grant, consumption, or job insertion.

Language-level `CanonicalHoldoutPeriod` and
`CanonicalHoldoutEvaluationPlan` values are convenience projections, not
authority tokens. Trust-sensitive approval, materialization, lease, and
dispatch paths obtain period bytes, plan bytes, schema digests, and referenced
artifact bytes from server-owned resolvers and strictly reparse them at the
boundary. They never accept a caller- or worker-constructed canonical wrapper
as proof. Phase 4 separately resolves and atomically consumes the persisted
grant; Phase 7 parses the exact referenced BacktestSpec bytes and binds every
materialized field to those bytes.

## Shared canonical JSON profile

Both documents use UTF-8 without a BOM, insignificant whitespace, or a trailing
newline. Objects contain exactly the fields shown below in the specified order.
Unknown, missing, duplicate, or out-of-order fields are invalid. JSON number
tokens, `null`, dynamic-key objects, and non-ASCII identifiers are forbidden.
Integral and decimal domain values are normalized strings.

All SHA-256 text values are `sha256:` followed by exactly 64 lowercase
hexadecimal characters. Hash inputs contain the shown ASCII domain, one NUL
byte, and the exact canonical bytes. They contain no trailing NUL or newline.

## Canonical holdout period

The exact document shape and field order is:

```json
{"schema":"loop.holdout-period/v1","sample":{"role":"first_locked_confirmation","start_inclusive":"2021-01-01","end_inclusive":"2024-12-31"},"snapshot_ids":["sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"],"snapshot_manifest_sha256":"sha256:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}
```

The `schema` value is exactly `loop.holdout-period/v1`. `sample` has exactly
`role`, `start_inclusive`, and `end_inclusive`, in that order. The only accepted
roles are:

- `first_locked_confirmation`; and
- `second_locked_historical_holdout`.

Dates are real proleptic-Gregorian civil dates in zero-padded `YYYY-MM-DD`
form. Years are from 1900 through 9999, and `start_inclusive` is not after
`end_inclusive`. Calendar dates do not imply that every date is a trading day;
the snapshot manifest and the frozen trading-calendar artifact own that rule.

`snapshot_ids` contains 1 through 128 strict SHA-256 identities, sorted by
unsigned ASCII byte order and unique. It names every immutable snapshot in the
manifest. `snapshot_manifest_sha256` is the raw digest identity of the exact
manifest bytes. A resolver independently verifies manifest membership and
sample-role consistency before constructing this domain value.

```text
canonical_period_digest = SHA-256(
  ASCII("loop.holdout-period/v1") || 0x00 || canonical_period_bytes
)

canonical_period_sha256 = "sha256:" + lower_hex(canonical_period_digest)
holdout_period_id        = canonical_period_sha256
```

The wire `Sha256Digest` contains the 32 raw digest bytes, while the wire
`HoldoutPeriodId.value` contains the textual value. Both must be recomputed and
must agree. A caller-supplied matching-looking alias is not accepted.

## Canonical holdout evaluation plan

The plan contains only immutable references. A complete canonical
`BacktestSpec` is stored as its own content-addressed artifact so the plan
format does not duplicate or weaken the backtest schema. Phase 7 defines and
validates the contents of that artifact; before a holdout approval, the plan
validator resolves it, verifies its raw digest, and requires its schema digest
to match the trusted backtest-schema registry.

The exact top-level shape and field order is:

```json
{"schema":"loop.holdout-evaluation-plan/v1","holdout_period_id":"sha256:2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","canonical_period_sha256":"sha256:2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","entries":[{"entry_index":"1","factor_spec_id":"sha256:3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","backtest_spec_artifact":{"artifact_id":"sha256:4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","uri":"artifact://sha256/4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","sha256":"sha256:4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","schema_name":"loop.backtest_spec","schema_version":"1","schema_sha256":"sha256:5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","media_type":"application/json","byte_size":"2048"},"job_budget":{"maximum_steps":"1","maximum_input_tokens":"1","maximum_output_tokens":"1","maximum_cost":{"amount":"1","currency_code":"USD"},"maximum_wall_time_ns":"60000000000"}}]}
```

The `schema` value is exactly `loop.holdout-evaluation-plan/v1`.
`holdout_period_id` and `canonical_period_sha256` must equal the independently
validated period identity. `entries` contains 1 through 4,096 values in
ascending `entry_index` order. Indices are normalized unsigned decimal strings,
start at `1`, and are contiguous without gaps.

Every entry has exactly `entry_index`, `factor_spec_id`,
`backtest_spec_artifact`, and `job_budget`, in that order. Factor IDs are strict
SHA-256 identities and unique within the plan. Backtest artifact IDs are also
unique so one frozen specification cannot be accidentally scheduled twice.

### Backtest specification artifact reference

The nested object has exactly these fields and order:

```text
artifact_id, uri, sha256, schema_name, schema_version, schema_sha256,
media_type, byte_size
```

`artifact_id` and `sha256` are the same strict SHA-256 text value. `uri` is
exactly `artifact://sha256/<the same lowercase hex digest>` and contains no
credential, query, or fragment. `schema_name` is exactly
`loop.backtest_spec`, `schema_version` is exactly `1`, and `media_type` is
exactly `application/json`. `schema_sha256` must equal the trusted immutable v1
backtest schema digest supplied by the schema registry. `byte_size` is a
normalized unsigned decimal string from `1` through `268435456` and must equal
the resolved artifact size. The corresponding wire `ArtifactRef` must omit
`row_count` and `manifest_sha256`; `created_at` is provenance metadata and is
not part of this canonical identity projection.

### Job budget

The nested object has exactly these fields and order:

```text
maximum_steps, maximum_input_tokens, maximum_output_tokens, maximum_cost,
maximum_wall_time_ns
```

The first three values and `maximum_wall_time_ns` are normalized unsigned
decimal strings. `maximum_steps` is from `1` through `1000000`; both token
limits are from `0` through `1000000000000`; and wall time is from `1` through
`604800000000000` nanoseconds (seven days). `maximum_cost` has exactly `amount`
then `currency_code`. `amount` uses the normalized non-negative decimal grammar
from `factor-canonicalization-v1.md`, has at most 18 significant digits and at
most 9 fractional digits, and is at most `1000000`. `currency_code` is exactly
three uppercase ASCII letters. A deployment policy may impose tighter limits,
but cannot reinterpret canonical bytes that meet these protocol bounds.

## Plan identities and reference validation

```text
plan_raw_digest = SHA-256(canonical_plan_bytes)

plan_sha256 = "sha256:" + lower_hex(plan_raw_digest)

holdout_evaluation_plan_id = "sha256:" + lower_hex(
  SHA-256(
    ASCII("loop.holdout-evaluation-plan/v1") || 0x00 ||
    canonical_plan_bytes
  )
)
```

The raw `plan_sha256` must equal the wire `canonical_plan.sha256`, its
`ArtifactId`, and its content-addressed locator. The artifact schema is exactly
`ArtifactSchemaReference{name = "loop.holdout_evaluation_plan", version = 1}`;
its schema digest must match the trusted plan-schema registry, its media type is
`application/json`, its byte size equals the canonical byte length, and it has
no row count or partition-manifest digest. The independently domain-separated
plan ID must equal `HoldoutEvaluationPlanReference.holdout_evaluation_plan_id`.
The decoded number of entries must equal `entry_count`.

Validation is performed from canonical bytes, not from an object that has
already discarded duplicate fields or unknown properties. Implementations
must strict-parse, validate, emit canonical bytes, and require byte equality
with the supplied artifact before computing either identity.

## Required conformance vectors

One shared fixture suite must prove identical bytes and identities in Rust,
TypeScript, and Python. It includes:

- valid examples for both locked roles and multi-entry plans;
- unsorted or duplicate snapshots, invalid dates, reversed windows, forbidden
  roles, aliases, and period ID/digest mismatches;
- empty, oversized, reordered, duplicate, or non-contiguous plan entries;
- duplicate factor and backtest identities;
- inline backtest objects, invalid content-address locators, schema or media
  mismatches, wrong trusted schema digests, byte-size mismatches, and unresolved
  artifact digests;
- malformed, negative, non-normalized, overflowed, or policy-exceeding budgets;
- unknown, missing, duplicate, or out-of-order JSON fields and excessive JSON
  nesting that must fail without a recursion crash;
- plan raw-digest, domain identity, period binding, and entry-count mismatches;
  and
- a mutation that is rehashed and re-signed but still fails its semantic
  invariant.

No validation failure is a factor rejection. It is a typed validation,
authorization, dependency, or integrity failure and performs no state mutation.
