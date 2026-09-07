# ADR 0004: Wire contracts and canonical research identities

- Status: Accepted
- Date: 2026-09-05
- Owner: hojiahao

## Context

Loop Engine has three implementation languages and several independently
deployable processes. They need an evolvable wire protocol, but research
identities must remain stable when a Protobuf runtime, generated binding, or
transport implementation changes.

Protobuf explicitly does not define serialization as canonical. Field order,
unknown fields, and runtime-specific re-encoding behavior make serialized
message bytes unsuitable as a research identity. The legacy engine also showed
the risk of computing an identifier before normalizing the expression that is
actually evaluated.

The contracts additionally cross a security boundary. Model-facing discovery
must never gain access to locked holdout data, and immutable research datasets
must not be copied through service RPCs.

## Decision

### Separate transport from identity

Versioned Protobuf messages are wire DTOs only. They carry validated domain
values between services, but their encoded bytes are never hashed to identify a
factor, policy, artifact, run, or result.

Research identities use the narrow canonical JSON profile in
`docs/specs/factor-canonicalization-v1.md`. In particular:

```text
expression_id = "sha256:" + lower_hex(
  SHA-256(ASCII("loop.factor-ast/v1") || 0x00 || canonical_ast_bytes)
)

factor_spec_id = "sha256:" + lower_hex(
  SHA-256(ASCII("loop.factor-spec/v1") || 0x00 || canonical_factor_spec_bytes)
)
```

The NUL byte makes the domain prefix unambiguous. Canonical identity documents
have schema-defined field order and reject unknown fields. They contain no JSON
floating-point numbers, object maps with dynamic keys, Protobuf `map`, `Any`, or
`Struct`. Identifiers are strict ASCII, and numerical values use normalized
decimal strings.

`expression_id` is computed only after canonicalization. `factor_spec_id`
commits, in order, to the expression identity, the exact operator-registry
digest, a frozen direction, and content-addressed references to every policy
that can alter research behavior. Labels, descriptions, admission status, and
measured performance are metadata and do not participate in either identity.

### Make normalization executable

Normalization may reorder or flatten arguments only when the versioned
operator registry explicitly marks that exact operator semantic version as
commutative or associative. The implementation does not infer algebraic laws
from an operator name and does not perform constant folding or approximate
numeric rewrites.

The evaluator executes the canonical tree, not the submitted tree. The stored
canonical bytes, parsed canonical tree, and executed tree must therefore be the
same semantic object. This is an enforced invariant rather than an export-time
cleanup step.

The AST representation is typed recursively and may describe a valid scalar or
enum subtree for editing, argument validation, and conformance tests. A factor
is stricter: before a canonical expression can be bound into `FactorSpec` or
sent to a factor evaluator, the registry must resolve its root type to
`series`. A decimal, Boolean, or enum root is not an executable factor.

### Keep FactorSpec immutable

A `FactorSpec` contains `higher_is_better` or `lower_is_better`; `auto`,
`best_ic`, and `best_icir` are not valid frozen directions. Direction selection
is an IS operation that must finish before the immutable specification is
created.

The specification includes the SHA-256 identity of the immutable operator
registry and content-addressed references for universe, data, calendar,
preprocessing, neutralization, portfolio, execution, cost, and evaluation
policies. The registry digest is an independent semantic input, not a tenth
policy. Replacing it or any referenced policy revision creates a new
`factor_spec_id`; existing metrics do not silently migrate to it.

Every registry operator also carries a required raw-content SHA-256 for a
closed `loop.operator-semantic-contract/v1` document. Construction resolves the
bytes, verifies their address and canonical encoding, and requires the bound
operator/version to match before producing an immutable registry snapshot.
The contract explicitly fixes null, window, tie, labeled-axis alignment, and
numeric behavior; every inapplicable dimension uses an explicit variant rather
than omission or a default. Declared behavior therefore participates in the
registry identity.

This semantic digest is not an implementation binary or source hash. Phase 6
must prove evaluator conformance with numerical goldens and property tests.
`ResearchProvenance.source_code_sha256` independently invalidates metrics when
executable source changes, avoiding platform-specific implementation hashes in
`FactorSpec`.

### Use explicit, bounded wire contracts

The shared `loop.v1` Protobuf package defines jobs, factors, data snapshots,
model content, streams, backtests, audit events, and artifact references.
Role-scoped services live in `loop.protocol.v1`, `loop.discovery.v1`,
`loop.provider.v1`, `loop.research.v1`, `loop.jobs.v1`, `loop.audit.v1`, and
`loop.holdout.v1`. Splitting service packages makes generated client surfaces
and authorization policy reviewable: discovery and provider clients do not
gain a holdout RPC merely because they use common DTOs.

Contracts use typed messages and `oneof` variants instead of generic JSON
containers. Every enum reserves zero for `*_UNSPECIFIED`. Every list is bounded
by API policy, every mutating request carries an idempotency key, and every
cost-incurring or mutating RPC has a deadline. Research service contracts accept
only narrow development-reference shapes and return a role-owned
`ResearchJobHandle`; their
request, response, and generated dependency graphs cannot reach the generic job
union, `BacktestSpec`, locked sample windows, or holdout contracts. Phase 3 maps
validated input into an internal durable job atomically. Because snapshot IDs
are opaque, Phase 2 wire validation proves only reference shape; the Phase 4/5
server-owned resolver and capability policy must prove the referenced sample
roles before persistence or execution. Factor evaluation,
backtesting, and reconciliation never execute inside a long-lived unary RPC.

RPC validation, authorization, cancellation, dependency, timeout, and
infrastructure failures use non-OK gRPC status with typed
`loop.v1.ServiceError` details. `FactorRejection` is instead a terminal domain
outcome for a valid, evaluated `FactorSpec`; it is never used to disguise an
invalid request or an operational failure and does not trigger infrastructure
retry policy.

Large data remains outside RPC. `ArtifactRef` identifies immutable bytes by
SHA-256 and carries a credential-free locator, media type, schema identity,
size, and optional row count. Receivers resolve credentials locally and verify
the digest before use.

Every submitted job pins a `ProtocolSelectionSnapshot`, including the selected
package, enabled features, effective limits, both peer builds, and the current
schema-descriptor digest. Research and backtest work additionally pins source
code, operator registry, configuration, data manifest, trading calendar,
environment, deterministic seed, and immutable backtest specification
identities. A freeze manifest records the repository-native VCS object ID and a
separate SHA-256 tree manifest; it does not pretend that every VCS uses SHA-256.

### Treat compatibility and preservation as different properties

An older Protobuf reader can skip a field it does not know. That does not imply
that every generated runtime preserves that field after decode and re-encode.
Loop Engine therefore requires old readers to tolerate additive fields, but it
does not rely on binding-level unknown-field retention. Components that must
forward or archive a message losslessly retain the original wire envelope as
immutable bytes alongside the parsed view.

Additive changes remain in the applicable v1 package; breaking shared DTO or
role-service changes require the corresponding new package major. Field numbers
are never reused, deleted fields and enum values are reserved, and supported
peers negotiate versions before submitting work. The detailed rules and test
matrix are in `docs/specs/protocol-compatibility.md`.

The generated `schema.current.binpb` is the descriptor for the source currently
checked out and must regenerate deterministically. The separately committed
`schema.baseline.binpb` is the explicit compatibility comparison trust anchor.
Normal generation and `--write` update only the current descriptor; baseline
creation is an explicit one-time action that refuses to overwrite an existing
baseline.
The accepted Phase 2 seed trust anchor, created after narrowing
the discovery, holdout, and research dependency surfaces, has SHA-256
`27a38398e290caee3fb857063c0f2adbbe43a7d2d44322103dbf4b72535be979`.
The unpublished safety-audit correction sequence and every rejected descriptor
are recorded in `docs/verification/phase-02-protocol-baseline.md` and
`fixtures/contracts/protocol/history/`. Its complete Phase 2 host and
clean-container exit gates passed for pushed implementation `0615d81`, as
recorded in `docs/verification/phase-02-core-contracts.md`. This acceptance is
not a product release or a research freeze.

### Keep capabilities out of domain messages

No Protobuf message contains a holdout capability or secret. A holdout
capability is a short-lived, audience-bound authorization value carried only in
protected RPC metadata and consumed by a server interceptor. Its value is
redacted before telemetry and never appears in an audit event; the audit event
records only the authorization decision and non-secret capability class.

Discovery request and response DTOs are structurally incapable of reaching a
holdout contract type. Their service package owns a development-only input,
returns only a narrow job handle, and has no transitive dependency on generic
job or holdout contracts. This descriptor property does not restrict arbitrary
RPC metadata. Phase 4 must enforce, with server interceptors and principals,
that discovery callers are never issued or permitted to present a holdout
capability. Provider role messages do not import holdout contracts, but model
content can carry a generic `ArtifactRef`; the type alone cannot prove what its
opaque locator names. Phase 9 must restrict providerd to prompt-safe schemas in
a separate artifact namespace, deny its runtime identity research and holdout
storage access, and test those denials. Until then, no runtime isolation claim
is made from descriptor reachability alone.

Research APIs follow the same structural rule. Their role package owns narrow
factor-evaluation, development-backtest, and reconciliation inputs plus a safe
job projection. `ReturnDefinition` and `ResearchProvenanceFingerprint` live in
the dependency-leaf `research_common.proto`, preserving their `loop.v1` names
without importing the `BacktestSpec` and its locked `SampleWindow`.
`DevelopmentDatasetReference` likewise lives alone in the dependency-leaf
`development_data.proto`, preserving its `loop.v1` name without importing the
broad `data.proto`. Consequently, neither the Research nor Discovery descriptor
closure loads `SampleRole`, `SampleWindow`, artifact-bearing `DataSnapshot`, or
holdout contracts. The development reference itself contains only snapshot IDs
and a manifest digest. That structural isolation does not prove what an opaque
ID names; server-owned snapshot resolution and capability enforcement are
mandatory Phase 4/5 prerequisites and fail closed when unavailable.

`HoldoutPeriodId` is the domain-separated SHA-256 of the canonical sample role,
inclusive window, sorted unique snapshot IDs, and snapshot-manifest digest; it
is not a caller-selected alias. The freeze manifest pins one content-addressed
canonical evaluation plan containing the complete ordered set of frozen
`FactorSpecId`, content-addressed canonical `BacktestSpec` artifact references,
and budget entries. The period and plan IDs, digests, and entry count are
repeated on approvals and grants and must match exactly. Exact bytes, field
order, limits, identity functions, and negative cases are defined in
`docs/specs/holdout-canonicalization-v1.md`.

Holdout approval is durable and attributable before any grant exists. One
approval RPC records one authenticated human's immutable approval; the server
derives the approver from transport identity and resolves the approval policy
itself. Grant creation verifies distinct, current approval records for the exact
period, freeze manifest, and evaluation plan. The period aggregate moves
monotonically from `SEALED` to `GRANT_ISSUED` exactly once and then to
`CONSUMED` or `CLOSED`. Expiry or revocation never reopens it. Consumption
accepts no caller-supplied factor, backtest specification, budget, or plan. One
transaction consumes the grant, advances the period, creates a batch, and
inserts all plan-derived internal jobs; the response exposes only IDs, counts,
revision, and creation time.

### Canonicalize audit events independently

Audit payload and append-chain hashes use dedicated canonical domain documents,
not Protobuf bytes, deterministic Protobuf output, or Protobuf JSON. The exact
field order, timestamp and enum spellings, domain separators, payload digest,
genesis value, and event-chain input are specified in
`docs/specs/audit-event-canonicalization-v1.md`. The event digest includes the
ledger identity and previous event digest, preventing reordering and
cross-ledger replay.

## Rejected alternatives

- Hashing deterministic Protobuf output was rejected because deterministic is
  not canonical across schema changes, languages, or runtimes.
- Hashing user-submitted expression text was rejected because equivalent input
  syntax can diverge from the tree that is stored or evaluated.
- Applying algebraic rewrites globally was rejected because missing values,
  overflow, operator definitions, and floating-point evaluation can invalidate
  ordinary mathematical identities.
- Embedding data or presigned URLs in messages was rejected because it expands
  the RPC and credential leakage boundary.
- Putting a reusable holdout token in a job record was rejected because jobs,
  logs, audit exports, and model context have wider readership than the locked
  dataset.
- Treating an expired or revoked holdout grant as permission to issue another
  grant was rejected because it would turn a nominally single-use historical
  period into a reusable development set.
- Accepting a backtest specification or budget during grant consumption was
  rejected because it lets the caller replace the batch that humans approved.
- Persisting an opaque holdout-period alias was rejected because two aliases
  could identify the same snapshots and bypass the single-use aggregate.
- Hashing serialized `AuditEvent` messages was rejected for the same
  non-canonical and cross-runtime reasons as factor identity hashing.

## Consequences

- Generated bindings may change without changing factor identity.
- Canonical schema evolution is intentionally stricter than Protobuf evolution
  and requires a new domain prefix when identity semantics change.
- Services must validate decoded DTOs before constructing domain objects.
- Lossless Protobuf forwarding requires storing original bytes, not merely
  reserializing a generated object.
- Policy revisions and operator semantic revisions naturally invalidate
  dependent metrics by producing new identities.
- Holdout access remains a control-plane authorization action rather than
  portable application state.
- Service-package separation provides a contract-level boundary that can be
  checked independently from deployment and transport authorization.
- Audit-chain verification remains stable across generated binding and wire
  schema changes.

## Enforcement

Phase 2 is accepted only when all supported language bindings pass:

1. canonical identity golden vectors, including negative and size-limit cases;
2. cross-language semantic round trips without asserting Protobuf byte
   equality;
3. old-reader/new-writer compatibility fixtures and unknown-field tolerance;
4. Buf lint and breaking-change checks against the repository baseline;
5. dependency checks proving holdout DTOs and contracts are unreachable from
   discovery request and response graphs;
6. artifact-reference validation proving credentials and inline datasets are
   rejected;
7. typed non-OK gRPC failure fixtures that cannot be decoded as factor
   rejection;
8. canonical audit payload/event hash-chain vectors, including tamper,
   reordering, and cross-ledger replay negatives; and
9. descriptor reachability and three-language binding tests proving holdout
   consumption cannot accept a caller-supplied plan, factor, backtest
   specification, or budget; and
10. three-language canonical holdout period and evaluation-plan vectors proving
    alias resistance, exact plan binding, and fail-closed artifact validation.

## References

- [Protocol Buffers: Proto Serialization Is Not Canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/)
- [Protocol Buffers: Updating A Message Type](https://protobuf.dev/programming-guides/proto3/#updating)
- [Protocol Buffers JSON mapping](https://protobuf.dev/programming-guides/json/)
- [Buf breaking change detection](https://buf.build/docs/breaking/)
- [gRPC versioning guide](https://grpc.io/docs/guides/versioning/)
