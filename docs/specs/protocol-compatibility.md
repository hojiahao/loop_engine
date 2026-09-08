# Core protocol compatibility and security policy

## Scope

This specification governs the versioned contracts exchanged by `loopd`,
`providerd`, `researchd`, `loopctl`, the Web client, and the TUI. The sole source
for generated wire types is the versioned `.proto` source under
`proto/loop/**/v1/`. `proto/loop/v1/` contains shared DTOs; role-specific
directories contain service entry points and their request/response envelopes.

Protobuf is a transport DTO format. Domain validation happens after decode, and
canonical research identities follow
`docs/specs/factor-canonicalization-v1.md`, never Protobuf bytes or Protobuf
JSON.

## Contract families

The shared `loop.v1` package defines provider-neutral, data-vendor-neutral typed
messages for:

| Family | Required semantics |
| --- | --- |
| Common | typed IDs, SHA-256 digests, exact decimals, timestamps, pagination, protocol errors, correlation and causation IDs |
| Artifact | immutable locator, digest, schema, media type, byte size, optional row count |
| Factor | typed AST DTO, canonical identity, frozen `FactorSpec`, policy references, validation failures |
| Data | stable security IDs, snapshot identity, lineage, `known_through`, entitlement/data grade, and sample role |
| Model | resolved model snapshot, typed content blocks, tool declarations/results, structured output schema, usage and budget |
| Stream | request ID, monotonically increasing sequence, event time, typed event body, and exactly one terminal event |
| Job | immutable submission, idempotency key, revision, lease, heartbeat, cancellation, terminal reason and failure class |
| Backtest | immutable specification/provenance, artifact references, normalized returns, metrics and reconciliation evidence |
| Audit | actor, action, target, correlation, causation, append-chain hashes, named override, reason and evidence references |
| Holdout | canonical period identity, immutable batch evaluation plan, per-person approvals, monotonic period record, single-use grant and narrow batch handle |

Infrastructure failure and deterministic factor rejection are distinct wire
variants. A timeout, unavailable dataset, corrupt artifact, provider failure, or
worker crash must not be encoded as a factor rejection.

### Role service entry points

Service packages are separate authorization and dependency surfaces:

| Package | Service | Runtime owner | Permitted role |
| --- | --- | --- | --- |
| `loop.protocol.v1` | `ProtocolService` | `loopd` | read build capabilities before mutable or paid work |
| `loop.discovery.v1` | `DiscoveryService` | `loopd` | submit bounded discovery against development dataset references only |
| `loop.provider.v1` | `ProviderService` | `providerd` | invoke or stream a resolved model; no research or holdout imports |
| `loop.research.v1` | `ResearchService` | `loopd` | enqueue development work for `researchd` workers |
| `loop.jobs.v1` | `JobService` | `loopd` | inspect, lease, heartbeat, complete, or cancel durable jobs |
| `loop.audit.v1` | `AuditService` | `loopd` | append through compare-and-swap and read an immutable audit ledger |
| `loop.holdout.v1` | `HoldoutService` | `loopd` | record approval, issue/consume the only grant, and atomically enqueue its frozen holdout batch |

Sharing `loop.v1` DTOs does not confer access to every service. Generated
clients and public TypeScript role entry points are assembled from explicit
allowlists. The holdout entry point exports only its RPC surface and the DTOs
reachable from that surface; it does not re-export generic job, budget,
backtest-specification, or internal job-input contracts. The research entry
point exports role-owned development inputs, its narrow job projection, and
only the exact shared DTOs needed to construct them. Its descriptor dependency
closure excludes `job.proto`, `backtest.proto`, the broad `data.proto`, and every
holdout file. `DevelopmentDatasetReference` resides alone in the safe
`development_data.proto` dependency leaf. The service message graph includes
`DevelopmentDatasetReference` but not `SampleRole` or `SampleWindow`. This is a
type-reachability boundary, not proof about the objects named by opaque IDs.
The Phase 4/5 server-owned capability and snapshot-registry resolver must prove
that every referenced snapshot has an allowed development role before
persistence or execution; unresolved or mismatched references fail closed.
Discovery and provider principals receive neither a holdout
client nor holdout authorization; Web, TUI, and CLI clients reach these roles
through authenticated `loopd` policy, not by bypassing the control plane.

## Protobuf authoring rules

1. Shared DTO files use package `loop.v1`; service envelopes use the exact
   role package `loop.<role>.v1`. All files use repository-controlled language
   namespaces.
2. Field numbers and enum numeric values are permanent. Removed members are
   marked `reserved` by number and name and are never reused.
3. Enum zero is named `*_UNSPECIFIED`. Receiving it where a choice is required
   fails validation; it is never treated as a useful default.
4. Presence-sensitive scalars use `optional` or a containing `oneof`; code must
   not infer presence from a scalar default.
5. Polymorphism uses a closed `oneof`. Generic `Any`, `Struct`, `Value`, and
   untyped JSON bags are forbidden in core contracts.
6. Protobuf `map` fields are forbidden. Typed repeated entry messages define a
   validation rule and deterministic semantic sort key where order is not
   meaningful.
7. Financial decimals, prices, quantities, rates, money, and budgets never use
   `float` or `double`. A typed exact-decimal message carries the normalized
   decimal string and, where applicable, an ISO currency or unit.
8. SHA-256 is carried as exactly 32 bytes in a typed digest message. Human-facing
   IDs use the validated `sha256:<lowercase-hex>` form.
9. Time instants use `google.protobuf.Timestamp` and are validated for the
   domain's precision and permitted range. Exchange-local calendar dates use a
   dedicated date message; they are not midnight timestamps.
10. Every mutable resource has a monotonic revision. Mutations require expected
    revision and idempotency key so retries cannot silently duplicate work.
11. Every repeated field has a documented maximum count. List APIs are paged
    with opaque, expiring page tokens and a server-capped page size.
12. Comments state units, presence semantics, sorting, limits, redaction, and
    whether a timestamp means event, knowledge, ingestion, or processing time.

Generated DTOs are not passed directly into domain or persistence layers. Each
service validates a DTO and converts it to a provider-neutral domain type. A
validation failure returns a typed protocol error and performs no state change.

## RPC failures and domain outcomes

RPC response bodies represent successful transport and command processing.
Validation, authentication, authorization, conflict, cancellation, rate limit,
budget, timeout, dependency, and internal failures return a non-OK gRPC status.
The status carries one allowlisted `loop.v1.ServiceError` rich-status detail at
type URL `type.googleapis.com/loop.v1.ServiceError`; its category, stable code,
retryable flag, and bounded redacted field details are validated before being
shown or persisted. Unknown error detail types and unrecognized categories fail
closed and are not guessed from status text.

`ServiceError.code` and every `ErrorDetail.code` use the ASCII grammar
`^[a-z][a-z0-9_.-]{0,127}$`. The top-level message and each detail message are
non-blank, contain no C0 or DEL control character, and are at most 2,048 UTF-8
bytes. A detail field path is non-blank, contains no ASCII control character,
and is at most 512 UTF-8 bytes. There are at most 32 details. The same control
character rule applies to bounded rejection and cancellation reasons. These
limits are applied after decode in every language; exceeding one is a protocol
failure, never a truncated or partially accepted error.

Validated text is still untrusted text. Log and UI sinks emit it only as an
encoded structured field and never interpolate it into a log record, terminal
escape sequence, HTML, SQL, or shell command. Secret redaction is enforced by
the allowlisted error constructor at the source; this boundary validates shape
and bounds and does not claim to infer whether arbitrary prose contains a
credential.

`loop.v1.FactorRejection` is different. It is a successful, deterministic
research determination about a syntactically valid, identity-verified
`FactorSpec`, recorded as a `JobOutcome`. It cannot represent a malformed
factor, corrupt artifact, unavailable dependency, worker crash, or provider
failure. Infrastructure retry policy reads `InfrastructureFailure`; it never
retries or admits based on a `FactorRejection` string.

### Job DTO validation and dispatch

Every Rust, TypeScript, and Python DTO boundary applies the same closed
`JobKind`/`JobSpecification.input` matrix. All seven v1 kinds, including
`REPORT` and `PROSPECTIVE_OBSERVATION`, are structurally valid only with their
declared input variant. Structural validity does not enable execution. Phase 2
exposes `validate_job_wire_dispatch_candidate`, a necessary but insufficient
wire gate: it validates the complete inline envelope and requires the selected
kind to be present in that process's explicit enabled-handler set, but its
returned kind neither selects nor authorizes a handler. FactorSpec registry
binding, holdout plan-entry binding plus the Phase 7 owning `BacktestSpec`
parser, artifact availability, server-owned dataset snapshot/capability
resolution, Phase 9 server-owned model-catalog resolution identity, and runtime
authorization are mandatory external gates. The Phase 2 model validator checks
only the snapshot's wire shape and digest lengths; it neither recomputes those
digests nor proves that the server catalog issued the resolution. A declared
but disabled kind fails as unsupported; an unknown enum, absent oneof, or
mismatched known variant fails before handler selection. Phase 2 intentionally
provides no single API that directly authorizes execution.

`JobRecord` validation is fail closed across state, attempt, lease, and outcome.
Queued records have attempt zero, and active records have one valid lease and a
positive attempt. Terminal records have no live lease and exactly the outcome
variant named by their state. Cancellation and budget exhaustion may have
attempt zero when work ends before the first lease; other terminal states
require a positive attempt. Phase 3 mutation boundaries must require
`jobs.prelease-terminal.v1` before exposing this behavior to mixed-version
peers. Administrative cancellation must not fabricate an execution attempt.
A lease binds the same
job ID and a non-future revision, identifies its owner, and has ordered issued,
heartbeat, and expiry timestamps. Terminal payloads validate their required
identity, enum, reason, timestamp, attempt, and collection limits.

A holdout job's `submitted_at` must fall inside its consumed grant's half-open
validity interval: `issued_at <= submitted_at < expires_at`. This is an inline
temporal-consistency check only. Phase 4 must still resolve the persisted grant,
verify that it is current and authorized, and consume it atomically; a valid
wire timestamp does not prove that the grant is unrevoked or unconsumed.

A factor rejection is permitted only for factor-evaluation, development
backtest, or internal holdout-backtest jobs. Its strict `FactorSpecId` must
equal the factor ID carried by that job's typed input. This wire/domain boundary
binding complements rather than replaces canonical factor verification: the
factor domain must first normalize and verify the expression and `FactorSpec`
identity, or resolve an already verified immutable `FactorSpecId`, before the
job is created. An operational failure uses a validated `ServiceError`, repeats
the record attempt, and can never be reinterpreted as `FactorRejection`.

The shared matrix in `tests/contracts/job_record_vectors.tsv` is executed in all
three languages and covers every declared kind, state/lease/outcome transitions,
factor-ID binding, terminal payload minima, collection limits, and unknown
enums. `validate_job_specification_shape` only classifies the closed kind/input
matrix. `validate_job_specification` adds all inline wire checks, and
`validate_job_wire_dispatch_candidate` maps those failures for a runtime
candidate boundary. None replaces the external gates above.

## Typed model content

Model messages use a `oneof` for text, image artifact reference, document
artifact reference, tool request, tool result, refusal, and schema-validated
structured output. Tool arguments and structured output are bounded canonical
JSON bytes paired with a schema ID and SHA-256; they are not `Struct` values.

Phase 2 defines these model and stream wire DTOs, but does not advertise an
executable model-content or terminal-stream feature. The provider-neutral
content validator is a Phase 9 exit condition; the consumer stream state
machine is a Phase 10 exit condition. Until those validators enforce missing
and unknown oneofs, enum values, canonical JSON identity and limits, request
binding, sequence, start/completion cardinality, and OK EOF behavior in every
supported language, `streams.terminal-event.v1` MUST NOT appear in negotiated
`ProtocolInfo.features`.

Model image, document, and tool-result blocks carry the generic `ArtifactRef`.
Its digest and schema fields do not prove that the referenced object is safe for
a prompt, nor do they prevent a locator from naming research or holdout data.
Phase 9 therefore requires a server-owned allowlist of prompt-safe schemas, a
separate artifact namespace, and a providerd runtime identity with no research
or holdout storage permission. Integration tests must reject disallowed schema
and namespace references before provider invocation. Phase 2 claims only DTO
and descriptor isolation, not this runtime ACL.

Each run records the resolved provider plugin, concrete model ID, model
capability snapshot, catalog digest, request policy, and pricing snapshot.
Provider-specific request extensions, when permitted, are typed at the provider
boundary and do not enter Loop or factor contracts as an unvalidated map.

## ArtifactRef contract

`ArtifactRef` describes immutable bytes; it never contains those bytes. It must
include:

- a content identity and 32-byte SHA-256 digest;
- a credential-free locator using an allowlisted scheme;
- exact byte size and media type;
- schema ID, schema version, and schema digest; and
- optional exact row count plus partition/manifest identity where applicable.

The URI must not contain user information, embedded credentials, bearer tokens,
session parameters, signatures, fragments, or presigned query strings. Object
store and local filesystem credentials are resolved from the receiving
service's narrowly scoped runtime identity. Local locators are resolved beneath
a configured artifact root after canonical path validation.

The receiver verifies the scheme, authorization, declared size, content digest,
and schema before opening an artifact. A mismatch is an infrastructure or data
integrity failure and fails closed. An existing artifact is never overwritten;
a correction creates a new content address and invalidates dependent results.

Datasets, Parquet tables, images, long documents, and replay ledgers cross
service boundaries only through `ArtifactRef`. The default RPC limits below are
not raised to transport large data.

## Limits and flow control

Unless a narrower method-level limit is documented, v1 applies these
application limits to uncompressed payloads:

- 4 MiB for one unary request or response;
- 1 MiB for one stream event;
- 256 KiB for canonical factor AST or structured tool JSON;
- 4,096 AST nodes and 64 levels of AST nesting;
- 500 records in one list page; and
- 128 bytes for an identity token and 2,048 bytes for a credential-free URI.

Transport limits are configured at least as strictly at both client and server.
Services validate decoded collection, nesting, and string limits before domain
construction. Compression does not increase an uncompressed application limit.

Mutating and cost-incurring requests require a caller deadline. Submission RPCs
enqueue bounded work and return promptly; long research execution is observed
through jobs and streams rather than an unbounded unary call. Cancellation is a
durable state transition, not merely a dropped connection.

`StartDiscovery` returns a `DiscoveryJobHandle` containing only the job ID,
projected status, revision, and submission/update timestamps. Its role package
owns the development-only `DiscoveryJobInput` and budget, so neither request nor
response can reach a generic job specification, holdout input, lease, or outcome
body. `EnqueueFactorEvaluation`, `EnqueueBacktest`, and
`EnqueueReconciliation` return a `ResearchJobHandle` with the same narrow
projection. Their role-owned inputs contain only narrow development-reference
shapes. The Phase 2 wire validator checks shape and identity syntax only; it
does not infer a sample role from an opaque snapshot ID. Before mapping a
request into an internal durable `JobSpecification`, loopd must use the
Phase 4/5 server-owned resolver and capability policy to prove every referenced
snapshot is allowed. The holdout consume RPC instead returns a narrow
`JobBatchHandle`; full plan-derived specifications remain inside loopd's durable
job store. None of these calls run LLM discovery, numerical evaluation,
backtesting, or reconciliation on the request thread.
Workers acquire a revision-checked lease, heartbeat it, and commit exactly one
typed terminal outcome. An RPC deadline expiring after enqueue does not cancel
durable work; the caller reconciles by idempotency key or returned IDs and
issues an explicit durable cancellation when authorized.

Once the Phase 10 stream feature is implemented, every stream event contains a
request or job ID and a sequence number beginning at one. Sequence numbers
increase by exactly one within a stream. A model stream succeeds only with one
`completed` event and no later event; cancellation, timeout, budget exhaustion,
and infrastructure failure terminate the RPC with a typed non-OK operational
status. A disconnect or OK EOF without completion is incomplete and must be
resumed or reconciled.

## Version negotiation

Peers expose protocol information containing their supported package majors,
minor feature set, size limits, and build identity. Clients negotiate before
submitting mutable or paid work and send the selected major in non-secret gRPC
metadata on every call.

Compatibility policy is:

- backward-compatible additions remain in the applicable v1 package;
- a peer must ignore an additive field it does not understand only when the
  enclosing operation remains safe without that field;
- a new field whose absence changes authorization, cost, execution, or research
  meaning requires an explicit feature gate or a new major service;
- breaking shared field/type changes use `loop.v2`; breaking role-service
  behavior uses that role's new major, such as `loop.research.v2`; and
- a server rejects an unsupported major or required feature before creating a
  job or spending budget.

Run state records the negotiated protocol, feature set, and generated-contract
build identity. A resumed run uses compatible semantics or fails closed; it
does not silently upgrade mid-run.

### Pinned protocol and research provenance

Every `JobSpecification` stores one `ProtocolSelectionSnapshot`, not merely a
floating package name. It pins the selected package, sorted enabled feature
set, effective limits, server and client build versions/digests, current schema
descriptor digest, selection time, and a digest of the canonical selection
document. Leasing and resume reject a selection whose pinned semantics are no
longer available; a running job never follows an upgraded `latest` alias.
The selection digest is calculated from a dedicated versioned canonical
document, never from serialized `ProtocolSelectionSnapshot` bytes.

Numerical jobs and `BacktestSpec` also bind a
`ResearchProvenanceFingerprint`: source-code tree, operator registry,
configuration, data manifest, trading calendar, and environment digests. The
backtest specification additionally binds a deterministic seed, frozen sample,
immutable snapshot IDs, factor specification, return definition, and its own
canonical specification digest. A result repeats the provenance and points to
immutable factor-value, position, order, fill, NAV, simple-return, exposure, and
cost artifacts. Any mismatch is an integrity failure and makes dependent
metrics stale; it is not patched in place.

Each provenance digest names canonical source, registry, configuration,
manifest, calendar, environment, or backtest-spec bytes defined by that
artifact's owning schema. Equal use of SHA-256 does not make those artifact
types substitutable, and no digest is derived from an incidental Protobuf
encoding.

Freeze manifests preserve two different source identities. `VcsObjectId`
records the repository-native commit algorithm and full object bytes (for
example Git SHA-1 or SHA-256), while `source_tree_sha256` records the canonical
path/mode/content manifest independently of VCS configuration. Data identity is
the immutable snapshot manifest digest; backtest identity is the canonical
backtest specification digest. These values are not interchangeable even when
they happen to use SHA-256.

## Unknown fields and variants

Protobuf binary readers can parse messages containing field numbers unknown to
their schema. This is tolerance, not a cross-runtime preservation guarantee.
Generated runtimes differ in whether unknown fields survive conversion to a
language object, cloning, JSON conversion, and reserialization.

Loop Engine therefore uses these rules:

1. Ordinary endpoints accept additive unknown fields only when version and
   feature negotiation says the operation is safe for the older reader.
2. Semantic decisions use known validated fields only. An unknown enum numeric
   value, unknown required feature, or unknown `oneof` alternative is not mapped
   to `UNSPECIFIED` or a default action; the operation fails closed as
   unsupported.
3. A proxy, durable queue, or audit component that promises lossless forwarding
   stores the original binary envelope unchanged with its digest. It does not
   rely on parse-and-reserialize preservation.
4. Protobuf JSON is not used for durable forwarding. JSON conversion can lose
   unknown fields and has different compatibility characteristics from binary
   Protobuf.
5. Compatibility tests verify old-reader tolerance and known-field semantics;
   they do not assert that all Rust, TypeScript, and Python bindings emit
   byte-identical Protobuf or retain every unknown field after reconstruction.

Even deterministic Protobuf serialization is not a canonical identity format.
Tests compare decoded domain values and canonical research IDs, not arbitrary
wire byte order.

## Security and secret boundary

Core messages contain no API key, cloud credential, database password, session
cookie, private key, raw bearer token, presigned URL, or holdout capability.
Opaque secret references may identify an administrator-configured credential,
but only the owning service resolves them and their values never enter audit,
telemetry, model prompts, or provider responses.

Sensitive gRPC metadata is stripped or redacted before logging. Trace spans use
request, job, actor, and policy IDs, not request bodies or authorization values.
Error details are allowlisted and scrubbed at the service boundary.

### Holdout capability

A holdout capability is:

- minted by the control plane only after the required freeze and approval;
- short-lived, single-purpose, audience-bound, run-bound, and non-transferable;
- carried only in protected RPC authorization metadata;
- consumed and removed by the server authorization interceptor; and
- absent from Protobuf messages, job state, artifacts, logs, traces, audit
  payloads, UI responses, model context, and provider traffic.

Audit records include the non-secret capability class, approver, target frozen
manifest, decision, and resulting run, but never the capability value or a
replayable derivative.

### Canonical holdout period identity

`HoldoutPeriodId` is not a caller-selected alias. The canonical period document
is UTF-8 JSON with no insignificant whitespace and this exact field order:

```json
{"schema":"loop.holdout-period/v1","sample":{"role":"<locked-sample-role>","start_inclusive":"YYYY-MM-DD","end_inclusive":"YYYY-MM-DD"},"snapshot_ids":["<snapshot-id>"],"snapshot_manifest_sha256":"sha256:<lowercase-hex>"}
```

The role is the exact validated `SampleRole` name and must be a locked sample
authorized by the freeze policy; warmup, IS, development, temporal-isolation,
and prospective roles are rejected. Dates are zero-padded civil dates; snapshot
IDs are non-empty, bytewise sorted, unique, and limited to 128. The manifest
digest covers the complete immutable snapshot manifest. Unknown or omitted
fields, an invalid window, duplicate IDs, and a manifest mismatch are rejected
before persistence.

```text
canonical_period_sha256 = SHA-256(
  ASCII("loop.holdout-period/v1") || 0x00 || canonical_period_bytes
)
holdout_period_id = "sha256:" + lower_hex(canonical_period_sha256)
```

Only loopd's validated freeze workflow may insert a period. No public RPC
accepts an inline `HoldoutPeriod` or exposes a create/register/upsert path.
Every request that directly names a period also supplies its canonical digest,
and the server resolves both together. Phase 3 persistence must make the ID and
digest individually unique, enforce their computed equality on insert, and use
both in compare-and-swap lookups. A matching opaque string is never sufficient.

### Frozen holdout evaluation plan

The freeze manifest contains exactly one immutable
`HoldoutEvaluationPlanReference`. Its artifact uses
`ArtifactSchemaReference{name = "loop.holdout_evaluation_plan", version = 1}`
and canonical document identity `loop.holdout-evaluation-plan/v1`. It binds the
canonical period ID and digest and contains the complete ordered batch. Every
entry has a one-based contiguous index, one frozen `FactorSpecId`, one
content-addressed complete canonical `BacktestSpec` artifact reference, and one
bounded `JobBudget`. The list contains between 1 and 4,096 entries. Unknown
fields, duplicate indices or factor/spec identities, incomplete references,
and malformed artifact envelopes are invalid. Phase 2 verifies each referenced
BacktestSpec artifact's locator, schema identity, size, and raw content digest;
it does not yet parse that artifact's owning schema. Phase 7 must additionally
parse the exact bytes and reject development or mismatched snapshot references,
factor bindings, return definitions, provenance, or other invalid BacktestSpec
semantics before approval, materialization, or dispatch. Exact plan field order,
value grammars, bounds, identity functions, and cross-language negative vectors
are normative in `holdout-canonicalization-v1.md`.

`plan_sha256` is SHA-256 of the exact canonical artifact bytes and must equal
both `canonical_plan.sha256` and the digest implied by its content-addressed
artifact ID and locator. Plan identity is independently domain separated:

```text
holdout_evaluation_plan_id = "sha256:" + lower_hex(
  SHA-256(ASCII("loop.holdout-evaluation-plan/v1") || 0x00 || canonical_plan_bytes)
)
```

Before approval, grant, consumption, or job creation, loopd must resolve the
plan and referenced artifacts under its service identity. The Phase 2 parser
validates the plan locator, schema, byte size, raw digest, domain-separated ID,
period binding, entry count, strict factor-ID shapes, referenced BacktestSpec
artifact envelopes, and budgets. FactorSpec resolution and the Phase 7 owning
BacktestSpec parser are additional mandatory gates. The latter must parse the
exact resolver-owned artifact bytes, derive their canonical identity, and bind
the complete materialized `frozen_backtest_spec`, including sample, snapshots,
provenance, return definition, and seed; Phase 2 currently binds only the plan
entry's factor ID, budget, and claimed canonical-spec digest. Neither a valid
SHA-256 string nor a matching artifact digest proves those domain semantics. Any
failure is a typed operational or validation error and performs no mutation.
Canonical holdout wrapper objects are untrusted convenience values, not
authority tokens. The job binder reparses period and plan bytes supplied by a
server-owned resolver and never accepts request- or worker-supplied resolver
context. It only binds factor ID, budget, and the canonical-spec digest. Phase 7
must use the exact referenced artifact bytes to derive or validate the complete
`frozen_backtest_spec` and bind sample, snapshots, return definition,
provenance, seed, and every other owning-schema field before approval,
materialization, or dispatch.

Approval itself is one durable record per authenticated human. The server
derives `approved_by` from the authenticated transport principal, requires the
descriptive command actor to match, and binds the record to one exact holdout
period, canonical period digest, freeze-manifest digest, evaluation-plan ID,
plan digest, and entry count. Grant issuance resolves persisted approval record
IDs, checks expiry, distinct actors, required roles, and exact freeze/period/
plan agreement against the server-loaded approval policy. It then inserts the
first grant and transitions the period in one transaction. A caller cannot
inline an approver, replace the plan, or provide a weaker policy.

The durable `HoldoutPeriodRecord` is monotonic:

```text
SEALED -> GRANT_ISSUED -> CONSUMED
                       -> CLOSED
```

`GRANT_ISSUED`, `CONSUMED`, and `CLOSED` permanently prohibit another grant.
An issued grant that expires or is revoked still occupies the period's only
grant slot. The consume request carries only the grant reference and expected
grant/period revisions; the removed caller-supplied `BacktestSpec` and budget
field numbers and names are reserved. In one transaction loopd marks the grant
consumed, advances the period to `CONSUMED`, creates one durable batch, parses
the frozen plan, and inserts every plan-derived internal holdout job. Either all
entries commit or none do. An idempotent replay returns the same narrow batch
handle and never creates another execution. Each internal job repeats the plan
ID, plan digest, batch ID, and one-based entry index so a worker can verify the
stored specification came from that exact plan.

Discovery principals are never issued holdout capabilities. Discovery client
bindings do not expose holdout RPC methods, discovery service identities are
denied at the authorization layer, and discovery jobs receive only IS or
development snapshot references. A caller cannot turn an ordinary snapshot ID
into holdout access because artifact resolution independently enforces sample
role and capability authorization.

## Audit canonicalization

`AuditPayload.payload_sha256`, `AuditEvent.event_sha256`, and the append-chain
rules are defined by
`docs/specs/audit-event-canonicalization-v1.md`. They hash dedicated canonical
domain documents with domain separators. They never hash serialized Protobuf,
deterministic Protobuf output, generated-language objects, or Protobuf JSON.
Services verify payload bytes and the previous ledger head before appending an
event through compare-and-swap.

## Compatibility workflow

Every protocol change follows this sequence:

1. Update the source `.proto` and validation specification in one review.
2. Run formatting and lint checks.
3. Run Buf breaking-change detection against the committed comparison baseline.
4. Regenerate Rust, TypeScript, and Python bindings with pinned tools.
5. Fail if regeneration changes committed output after a clean second pass.
6. Run golden fixtures through every supported language.
7. Run old-reader/new-writer and new-reader/old-writer semantic tests.
8. Run negative validation, limits, malformed input, redaction, and capability
   boundary tests.
9. Review generated diff and reserve every removed field or enum value.
10. Commit the schema, bindings, current descriptor, fixtures, and compatibility
    evidence together. Advance the compatibility baseline only in a separately
    reviewed protocol release.

`fixtures/contracts/protocol/v1/schema.current.binpb` is regenerated from the
checked-out `.proto` source and must match it byte-for-byte on a clean second
generation. `schema.baseline.binpb` is the immutable comparison point for Buf
breaking checks. Ordinary generation updates `schema.current.binpb` only.
Baseline initialization is explicit and refuses to overwrite an existing file.
`scripts/verify-protocol-baseline.sh` additionally pins the baseline digest and
proves on a tampered temporary copy that the guard fails closed. Advancing the
baseline requires a reviewed release operation that updates this explicit trust
anchor, never an automatic generator side effect.

The accepted Phase 2 seed baseline was created only after
removing generic-job reachability from discovery and research, removing the
caller-controlled holdout specification path, and splitting development-safe
research provenance from `backtest.proto`. A subsequent boundary correction
moved `DevelopmentDatasetReference` into the dependency-leaf
`development_data.proto`, so Discovery and Research no longer load the broad
`data.proto` module that defines locked sample roles and full snapshots. Its
descriptor SHA-256 is
`27a38398e290caee3fb857063c0f2adbbe43a7d2d44322103dbf4b72535be979`.
The complete unpublished correction sequence is recorded in
`docs/verification/phase-02-protocol-baseline.md`; rejected descriptors are
content-addressed under `fixtures/contracts/protocol/history/` and verified by
the baseline guard. The full Phase 2 host, clean-container, and remote CI gates
passed for pushed implementation `0615d81`; see
`docs/verification/phase-02-core-contracts.md` for the acceptance evidence.

Because this is the first accepted seed baseline, not a released-client migration,
`schema.baseline.binpb` currently equals `schema.current.binpb`. The resulting
Buf check proves deterministic self-compatibility and establishes a guard for
future evolution; it does not demonstrate migration from a previously released
wire contract.

### Contract test matrix and later integration gates

| Test | Rust | TypeScript | Python | Required result |
| --- | ---: | ---: | ---: | --- |
| Current golden decode/encode | yes | yes | yes | Equal validated domain values |
| Canonical identity vectors | yes | yes | yes | Byte-identical canonical JSON and IDs |
| Older fixture read by current code | yes | yes | yes | Preserved known semantics |
| Current additive fixture read by older code | yes | yes | yes | Safe known subset or explicit unsupported result |
| Injected unknown field | yes | yes | yes | No crash; no unsafe default |
| Unknown enum/oneof semantic variant | yes | yes | yes | Fail closed as unsupported |
| Oversize/depth/count boundaries | yes | yes | yes | Reject before side effects |
| Artifact locator with credential/query | yes | yes | yes | Reject and redact |
| Discovery type reachability | yes | yes | yes | No job specification, holdout/grant/approval, or `BacktestSpec` is reachable |
| Discovery attempts holdout access | deferred | deferred | deferred | Phase 4/5 runtime authorization denied and no artifact opened; Phase 9 provider artifact ACL denied |
| Research dependency closure | yes | yes | yes | No generic job, backtest-specification, locked sample, or holdout file/type is reachable |
| Research public role exports | yes | yes | yes | Only development inputs, safe shared DTOs, and a narrow job projection are exported |
| Holdout frozen-plan surface | yes | yes | yes | Freeze pins one plan; consume accepts only grant/revisions and returns a narrow batch |
| Holdout descriptor reachability | schema | schema | schema | No factor, backtest, budget, generic job, or internal holdout job is reachable from RPCs |
| Infrastructure failure vs rejection | deferred | deferred | deferred | Phase 3/4/10 RPC and state-machine integration produces distinct typed terminal outcomes |
| Current descriptor regeneration | generated | generated | generated | Source and committed current descriptor match |
| Breaking check against baseline | schema | schema | schema | Baseline is not modified by generation |
| Audit payload/event chain | yes | yes | yes | Canonical digest, tamper and replay results agree |

Phase 2 requires every row marked `yes`, `schema`, or `generated`. Rows marked
`deferred` are explicit exit gates for the named later phases: Phase 2 defines
and validates the DTO/error boundary but does not claim runtime authorization,
artifact access control, or end-to-end RPC state transitions before those
components exist.

Fixtures include producer version, schema digest, expected domain projection,
and whether lossless original-byte forwarding is required. Golden data contains
no live credential or licensed market data.

## Change classifications

Generally compatible after tests and negotiation:

- adding an optional field whose absence preserves safe semantics;
- adding a new RPC without changing an existing method;
- adding an enum value when every older consumer safely rejects an unknown
  value; and
- relaxing a validation limit only when both peers advertise the new limit.

Breaking or requiring a new gated feature/major:

- changing a field number, wire type, meaning, unit, required presence, or
  redaction classification;
- reusing a removed number or enum value;
- adding a `oneof` alternative that an old consumer could mistake for an empty
  valid operation;
- changing decimal normalization, canonical identity, operator semantics, or
  execution timing;
- making a formerly optional authorization or cost field semantically required;
  and
- changing an RPC from idempotent to non-idempotent or altering terminal-state
  meaning.

## Authoritative references

- [Protocol Buffers: Updating A Message Type](https://protobuf.dev/programming-guides/proto3/#updating)
- [Protocol Buffers: Unknown Fields](https://protobuf.dev/programming-guides/proto3/#unknowns)
- [Protocol Buffers: Proto Serialization Is Not Canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/)
- [Protocol Buffers JSON mapping](https://protobuf.dev/programming-guides/json/)
- [Buf lint](https://buf.build/docs/lint/)
- [Buf breaking change detection](https://buf.build/docs/breaking/)
- [gRPC versioning guide](https://grpc.io/docs/guides/versioning/)
