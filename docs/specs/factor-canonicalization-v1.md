# Factor canonicalization v1

## Purpose

This specification defines the only byte representation used to identify a
Loop Engine factor expression or frozen factor specification. Protobuf, source
text, database rows, and language-native object serialization are not identity
formats.

The goals are:

- the same accepted domain value has the same bytes in Rust, TypeScript, and
  Python;
- no identifier is computed for a tree different from the tree executed;
- operator rewrites are explicit, versioned, and reviewable; and
- an operator-registry or research-policy change necessarily produces a new
  frozen factor identity.

## Identity functions

The external form of an identity is `sha256:` followed by exactly 64 lowercase
hexadecimal characters.

```text
expression_id = "sha256:" + lower_hex(
  SHA-256(ASCII("loop.factor-ast/v1") || 0x00 || canonical_ast_bytes)
)

operator_registry_sha256 = "sha256:" + lower_hex(
  SHA-256(ASCII("loop.operator-registry/v1") || 0x00 || canonical_registry_bytes)
)

factor_spec_id = "sha256:" + lower_hex(
  SHA-256(ASCII("loop.factor-spec/v1") || 0x00 || canonical_factor_spec_bytes)
)
```

The prefix and separator bytes shown above are part of the hash input. No BOM,
terminating newline, or terminating NUL follows the canonical JSON bytes.

## Canonical JSON profile

This is a closed, schema-specific format. It is not a promise to canonicalize
arbitrary JSON.

1. The encoding is UTF-8 without a BOM.
2. Objects emit fields in the order listed by this specification. Consumers
   reject duplicate, missing required, out-of-order, and unknown fields.
3. There is no insignificant whitespace before, between, or after tokens.
4. JSON floating-point and integer number tokens are forbidden. Numerical
   domain values are strings in the normalized decimal format below.
5. `null` is forbidden. An absent optional concept is represented by a
   schema-defined variant, never by an omitted ad hoc key.
6. Dynamic-key objects are forbidden. Key/value collections use typed arrays
   with an explicitly defined sort order.
7. All identity-bearing identifiers are ASCII and must match the grammar
   declared for their field. Unicode normalization is never implicit.
8. Boolean literals, where allowed by a node schema, are the JSON tokens
   `true` and `false`.
9. Arrays preserve semantic order except for the explicit operator transforms
   in this specification.
10. Implementations must parse into the typed domain model, validate limits,
    canonicalize the model, serialize it with the dedicated canonical writer,
    parse those bytes again, and execute that parsed canonical tree.

Generic JSON serializers must not be used to create identity bytes. Protobuf
JSON, `map`, `Any`, and `Struct` are not accepted as identity inputs.

### ASCII identifiers

Operator names, field names, enum types, and enum values use dot-qualified
tokens:

```text
identifier = segment *( "." segment )
segment    = lowercase *( lowercase / digit / "_" )
lowercase  = %x61-7A
digit      = %x30-39
```

Each identifier is 1 through 128 bytes. Policy IDs match
`^[a-z][a-z0-9_.-]{0,127}$`. A policy revision is a normalized unsigned decimal
integer string in `1..=18446744073709551615`, matching the `uint64`-bounded job
and persistence contracts.

`operator_version` uses the same positive `uint64` range and string encoding. It
identifies an immutable operator semantic revision; it is not a JSON number and
is not silently substituted by a newer revision. Values longer than 20 digits
and values above the maximum are rejected before hashing.

Uppercase letters, whitespace, empty segments, leading digits in identifiers,
path separators, percent encoding, and non-ASCII lookalikes are rejected.

### Decimal strings

A normalized signed decimal matches:

```text
^-?(0|[1-9][0-9]*)(\.[0-9]*[1-9])?$
```

An unsigned decimal omits the optional minus sign. Therefore `0` is the only
zero, an integer has no decimal point, and fractional values have neither
leading integer zeros nor trailing fractional zeros. Exponents, a leading plus,
`.5`, `1.`, `01`, `1.0`, `-0`, `NaN`, and infinities are rejected.

Each operator signature declares maximum precision, scale, and range. The
canonicalizer must parse with an exact decimal implementation and must never
round binary floating-point input into this format.

## Canonical AST

An AST node is exactly one of the following closed variants. The examples also
define object field order.

Field reference:

```json
{"node":"field","field":"market.close"}
```

Decimal literal:

```json
{"node":"decimal","value":"20"}
```

Boolean literal:

```json
{"node":"boolean","value":true}
```

Closed enum literal:

```json
{"node":"enum","enum_type":"rank.method","value":"average"}
```

Operator call:

```json
{"node":"call","operator":"rolling.mean","operator_version":"1","arguments":[{"node":"field","field":"market.close"},{"node":"decimal","value":"20"}]}
```

The permitted field names, enum domains, operator signatures, arity, argument
types, decimal limits, lookback behavior, missing-value behavior, and output
type come from the versioned operator registry. A call is invalid unless the
exact `(operator, operator_version)` entry exists.

Every node resolves to exactly one registry type: `series`, `decimal`,
`boolean`, or one named closed enum type. Scalar and enum nodes are valid typed
subtrees because they are needed as operator arguments and by AST editors and
conformance tooling. They are not complete factors. A canonical expression may
be stored independently, but binding it into a `FactorSpec` or submitting it to
the factor evaluator additionally requires the root to resolve to `series`.
The evaluator repeats that check after strict parse; it never coerces a scalar
root, broadcasts it across securities, or infers a type from the Protobuf
shape.

The v1 safety limits are:

- at most 4,096 AST nodes;
- at most 64 levels of nesting after canonicalization;
- at most 256 KiB of canonical AST bytes; and
- at most 1,024 direct arguments to a variadic call.

Limits are checked before hashing and again before evaluation. A rejected input
has no `expression_id`.

## Normalization algorithm

Implementations perform these steps in order:

1. Parse source syntax into a typed AST without assigning an identity.
2. Resolve every node type, field, enum, and operator against one immutable
   registry snapshot.
3. Normalize every decimal and reject lossy or out-of-range values.
4. Recursively canonicalize child nodes.
5. For an operator registry entry marked `associative: true`, flatten nested
   calls only when operator name and semantic version are identical. Preserve
   the left-to-right argument order at this step.
6. For an entry marked `commutative: true`, sort direct arguments by their full
   canonical node bytes using unsigned bytewise lexicographic order. Equal
   arguments remain repeated; canonicalization never deduplicates them.
7. Validate the normalized arity, type, depth, node count, and byte limits.
8. Serialize with the canonical writer and compute `expression_id`.
9. Parse the emitted bytes back into the canonical domain type and resolve its
   type again. When the expression is used as a factor, require a `series` root
   and make that parsed tree the evaluator input.

An operator receives neither transformation unless its exact registry entry
opts in. Associativity and commutativity are independent flags. The registry
must not opt an operator into a rewrite merely because the identity holds over
real arithmetic: missing-value propagation, overflow, rounding, evaluation
order, or domain errors may make the implemented operation different.

The v1 algorithm performs no constant folding, identity elimination, inverse
rewrites, distributive rewrites, field aliasing, unit conversion, approximate
comparison, or version substitution.

### Operator policy example

This example is descriptive registry data, not part of an AST identity:

```json
{
  "operator": "arithmetic.add",
  "operator_version": "1",
  "associative": false,
  "commutative": true,
  "reason": "Argument order is observationally irrelevant for this exact evaluator implementation."
}
```

If a later evaluator changes rounding or missing-value semantics, it receives a
new `operator_version`. Existing expression identities continue to resolve to
the old semantics or are reported unavailable; they are never reinterpreted.

## Operator semantic contracts

Every operator definition names a content-addressed semantic contract. Its
address is a raw content digest rather than a domain-separated research ID:

```text
semantic_contract_sha256 = "sha256:" + lower_hex(
  SHA-256(canonical_semantic_contract_bytes)
)
```

The closed contract emits exactly these fields in this order:

```text
schema, operator, operatorVersion, nullPolicy, windowPolicy, tiePolicy,
alignmentPolicy, numericPolicy
```

`schema` is exactly `loop.operator-semantic-contract/v1`. `operator` and the
positive-integer `operatorVersion` must equal the registry entry referring to
the contract. Every policy is required and uses a closed string variant below.
An inapplicable dimension is the explicit `not_applicable` variant; `null`,
omission, empty strings, and implementation-selected defaults are invalid.

Registry construction receives a content resolver, not trusted parsed
metadata. For every operator it resolves bytes by `semanticContractSha256`,
hashes the bytes, parses the closed schema, rewrites it with the dedicated
writer, requires exact byte equality, and verifies the operator/version
binding. Missing content, a false address, non-canonical bytes, or a contract
for another operator fails construction. Only the resulting immutable snapshot
has a registry identity.

### Null policy variants

- `not_applicable`: no nullable series observation participates in this
  semantic dimension; it does not authorize an evaluator-selected behavior.
- `propagate`: a missing required series operand at a coordinate makes that
  coordinate's result missing.
- `ignore_missing`: exclude missing observations from the population; the
  window policy still determines whether enough valid observations remain.
- `preserve_target_ignore_peers`: a missing target/current observation makes
  the result missing, while missing non-target observations are excluded.
- `reject_missing`: a missing required observation is a domain error, never an
  implicit fill or a missing output.

### Window policy variants

- `not_applicable`: no temporal window or lag is applied.
- `trailing_argument_2_full_window_right_inclusive_constant_preserve`: argument
  2 is positive width `n`; use the current timestamp and preceding `n-1`
  ordered timestamps for the same security. Output requires `n` valid
  observations. A constant valid window is evaluated normally and is not made
  missing merely because it is constant.
- `trailing_argument_2_minimum_valid_min_n_max_3_floor_2n_div_3_right_inclusive_constant_preserve`:
  use the same right-inclusive trailing `n` timestamps and require at least
  `min(n, max(3, floor(2*n/3)))` valid observations. Missing peers are excluded
  only when the null policy permits it. A constant qualifying window is
  evaluated normally unless a numeric or tie rule below defines its result.
- `lag_argument_2`: argument 2 is positive `n`; at timestamp `t`, use the same
  security's value at the `n`th preceding timestamp. Calendar interpolation
  and forward fill are forbidden.

### Tie policy variants

- `not_applicable`: equal-value ordering cannot affect the result.
- `average_valid_count`: equal values receive their average 1-based ordinal;
  divide it by the count of valid observations.
- `dense_valid_count`: equal values share a 1-based dense rank, with no gap
  after a tie; divide it by the count of valid observations.
- `stable_first_valid_count_minus_one`: break ties by stable input order, use a
  zero-based ordinal, and divide by `N-1`; `N < 2` produces missing.
- `argument_2_average_or_dense_valid_count_constant_midpoint`: argument 2
  selects `average` or `dense`. Subtract one from the selected 1-based rank and
  divide by `N-1`, where `N` is the valid population. `N <= 1` or an all-equal
  population yields exactly `0.5`.
- `target_last_stable_order_valid_count_minus_one_constant_midpoint`: rank only
  the target at the right edge of a trailing window. Remove missing peers,
  retain a missing target as missing, and use stable ascending order for its
  zero-based position divided by `N-1`. `N <= 1` or all valid values equal
  yields exactly `0.5`.

### Alignment policy variants

- `not_applicable`: labeled-series alignment does not apply.
- `unary_preserve_timestamp_and_security`: output preserves the input's exact
  timestamp/security labels, order, and cardinality.
- `strict_timestamp_and_security`: all series operands require identical
  timestamp/security labels in identical order; mismatch is a domain error and
  output preserves those axes.
- `intersection_timestamp_and_security`: evaluate only both label-set
  intersections in first-operand relative order. Union, filling, and label
  coercion are forbidden.

### Numeric policy variants

- `not_applicable`: no numeric result participates in this dimension.
- `exact_decimal`: use the canonical base-ten value without binary conversion,
  rounding, overflow, `NaN`, or infinity.
- `binary64_non_finite_to_missing`: use IEEE-754 binary64; normalize any result
  that is `NaN` or positive/negative infinity to missing before downstream use.
- `binary64_reject_non_finite`: use IEEE-754 binary64, but any non-finite input
  or result is a domain error.
- `ordinal_unit_interval`: every non-missing result is finite and in `[0, 1]`
  under the associated tie rule.
- `binary64_adjusted_fisher_pearson_effective_n_minimum_3_constant_zero_non_finite_to_missing`:
  for the `N` valid observations compute population moments
  `m2=sum((x-mean)^2)/N` and `m3=sum((x-mean)^3)/N`. `N < 3` yields missing;
  `m2 = 0` yields exactly `0`; otherwise output
  `sqrt(N*(N-1))/(N-2) * m3/(m2^(3/2))`. `N` is the valid count, never the
  configured window width. Negative `m2` from cancellation is clipped to zero,
  and a non-finite output becomes missing.

The digest binds declared behavior, but registry construction does not prove a
numerical implementation. Phase 6 must expose the read-only resolved contract
to evaluators and prove conformance with cross-language goldens,
missing/constant-window properties, and reference numerical comparisons.
`ResearchProvenance.source_code_sha256` separately binds executable source; a
source change makes prior metrics stale even when the contract and FactorSpec
remain unchanged.

## Canonical operator registry

An operator registry identity is derived from its validated snapshot; callers
never supply the identity independently. The registry canonical object has
exactly these fields and this order:

```text
schema, fields, enums, operators
```

`schema` is exactly `loop.operator-registry/v1`. `fields` is sorted by unsigned
ASCII `field`; each entry emits exactly `field, outputType`. `enums` is sorted
by unsigned ASCII `enumType`; each entry emits exactly `enumType, values`, and
`values` is sorted by unsigned ASCII value. `operators` is sorted first by
unsigned ASCII `operator`, then by the numerical value of the canonical
positive-integer `operatorVersion` (digit count, then ASCII bytes).

A value type is exactly the JSON string `series`, `decimal`, or `boolean`, or
the closed object `{"enumType":"<identifier>"}`. An argument emits `type`, then
`literalOnly` only when true, then `decimal` when present. A decimal constraint
emits exactly `maxPrecision, maxScale, minimum, maximum`; all four values are
canonical decimal strings, including the positive-integer precision and scale
limits.

An operator entry emits fields in this order:

```text
operator, operatorVersion, semanticContractSha256, parameters,
[variadic, minArguments, maxArguments],
outputType, associative, commutative
```

The bracketed fields are present together only for a variadic signature.
`semanticContractSha256` is required and resolves under the validation process
above. `minArguments` and `maxArguments` are canonical positive-integer strings in
identity bytes. `parameters` preserves declared positional order. Both policy
flags are mandatory JSON booleans. All objects are closed: unknown, duplicate,
missing, or out-of-order fields, unknown enum types, duplicate definitions, and
invalid signatures are rejected before canonical bytes or an identity exist.

The canonical writer then applies the `loop.operator-registry/v1` domain prefix
defined above. A `FactorSpec` binder constant-time compares its claimed
`operator_registry_sha256` with this computed identity before it may construct
a bound, hashable factor specification.

## Canonical FactorSpec

`FactorSpec` binds an expression to the frozen choices required to evaluate and
trade it. Its canonical object has exactly these fields and this order:

```json
{"schema":"loop.factor-spec/v1","expression_id":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","operator_registry_sha256":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","direction":"higher_is_better","universe_policy":{"policy_id":"us_common_stock","revision":"1","sha256":"sha256:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"data_policy":{"policy_id":"pit_market_v1","revision":"1","sha256":"sha256:2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"calendar_policy":{"policy_id":"xnys_xnas","revision":"1","sha256":"sha256:3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"preprocess_policy":{"policy_id":"cross_section_v1","revision":"1","sha256":"sha256:4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"neutralization_policy":{"policy_id":"industry_size_beta","revision":"1","sha256":"sha256:5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"portfolio_policy":{"policy_id":"decile_long_short","revision":"1","sha256":"sha256:6123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"execution_policy":{"policy_id":"next_tradable_open","revision":"1","sha256":"sha256:7123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"cost_policy":{"policy_id":"us_equities_cost_v1","revision":"1","sha256":"sha256:8123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"evaluation_policy":{"policy_id":"factor_admission_v1","revision":"1","sha256":"sha256:9123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}}
```

`operator_registry_sha256` immediately follows `expression_id`. It is the
`sha256:<lowercase-hex>` identity of the exact immutable registry used to type,
normalize, and execute the expression. It commits to operator semantic
versions, signatures, output types, decimal constraints, missing-value rules,
lookback rules, and the permitted associative/commutative transforms. It is not
a `PolicyReference`, does not have a mutable revision alias, and does not count
as a tenth research policy.

A policy reference always emits fields in this order:

```text
policy_id, revision, sha256
```

The `sha256` is the digest of the immutable policy artifact after that policy's
own schema validation. A resolver must verify it before evaluation.

`direction` is exactly `higher_is_better` or `lower_is_better`. Direction modes
that inspect validation, confirmation, holdout, or future results are invalid.
The IS selection event is audit metadata; the selected direction is the only
value admitted to the frozen specification.

The canonical FactorSpec intentionally excludes display name, prose rationale,
creator, timestamps, run ID, admission state, metrics, review text, and legacy
hashes. These values belong in linked metadata and audit records. It also
excludes the AST bytes themselves: `expression_id` resolves to separately
stored canonical AST bytes, whose digest and `series` root type must be verified
under the registry identified by `operator_registry_sha256` before use.

## Storage and verification

For every expression, storage keeps:

- canonical AST bytes under the `expression_id` content address;
- the parsed canonical AST or an index derived from it;
- the operator registry snapshot identity and resolved root type; and
- optional submitted syntax as non-authoritative evidence.

For every factor specification, storage keeps its canonical bytes under the
`factor_spec_id` and resolves every referenced artifact by verified digest.

On read, the service recomputes the relevant identity before constructing a
domain object. Constructing an executable factor also verifies that the
resolved registry digest equals `operator_registry_sha256` and that the strict
reparse resolves the root to `series`. A digest or type mismatch is an
integrity/validation failure, not a factor rejection, and fails closed.

## Required conformance vectors

The committed fixture suite must include:

- identical golden bytes and IDs in Rust, TypeScript, and Python;
- accepted decimal boundaries and every forbidden decimal form;
- positive-integer minimum/`uint64` maximum boundaries and overflow rejection;
- non-ASCII, mixed-case, path-like, overlength, and unknown identifiers;
- commutative permutations that converge to one identity;
- operators without an explicit policy whose permutations remain distinct;
- associative flattening restricted to one exact semantic version;
- missing-value-sensitive operators that are never algebraically rewritten;
- oversized, too-deep, unknown-field, duplicate-field, and malformed JSON;
- valid scalar and enum subtrees plus rejection of every non-`series` factor
  root at `FactorSpec` binding and evaluator entry;
- both frozen directions and all forbidden automatic direction modes;
- exact canonical operator-registry bytes and identity in every language;
- exact semantic-contract bytes and raw content hashes in every language,
  including closed-schema, explicit-NA, deep-JSON, unresolved-address,
  misaddressed-content, and operator/version-mismatch failures;
- a claimed operator-registry digest mismatch failing closed at binding;
- a legitimately changed registry snapshot producing a new registry identity;
- one changed policy digest producing a different `factor_spec_id`; and
- a proof that the evaluator receives bytes-equivalent canonical structure,
  rather than the submitted pre-normalized tree.
