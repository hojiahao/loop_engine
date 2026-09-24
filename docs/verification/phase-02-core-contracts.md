# Phase 2 verification: core contracts

- Date: 2026-09-07 (Asia/Shanghai)
- Branch: `refactor/us-equities-loop-runtime`
- Status: complete; implementation `0615d81` pushed and all exit gates passed
- Decision: `docs/adr/0004-core-protocol-and-canonical-identities.md`
- Protocol baseline history: `docs/verification/phase-02-protocol-baseline.md`

## Delivered contract surface

Phase 2 defines 19 Protobuf source files. Shared DTOs live in `loop.v1`, while
role-specific service entry points live in separately generated Discovery,
Provider, Research, Jobs, Audit, Holdout, and Protocol packages. Long-running
research RPCs return narrow job handles; the contracts do not claim that a
durable queue exists before Phase 3.

Generation currently produces:

| Language | Generated source files | Hand-written boundary layer |
| --- | ---: | --- |
| Rust | 8 | `crates/loop-protocol` and `crates/loop-core` |
| TypeScript | 19 | `packages/protocol-ts` |
| Python | 73 | `python/loop_protocol` |

The checked-in bindings are generated deterministically by
`scripts/proto-generate.sh`. Normal generation may update only the current
descriptor; it cannot overwrite the explicit compatibility baseline.

## Canonical identities

`loop-core` and both client-language packages implement the same closed
canonical profiles for:

- typed factor ASTs and immutable `FactorSpec` identities;
- versioned operator semantic contracts and registry snapshots;
- holdout period and frozen batch-plan identities;
- audit payloads and append-chain event identities; and
- validated immutable artifact references.

An expression is normalized before its SHA-256 identity is computed. Algebraic
flattening or ordering is enabled only by the exact operator semantic contract,
and an executable factor must resolve to a `series` root. A frozen factor
direction cannot use an adaptive `auto`, `best_ic`, or `best_icir` mode. The
factor identity binds the operator registry plus all nine behavior-changing
policy references.

Canonical IDs use a versioned ASCII domain separator, NUL delimiter, canonical
JSON bytes, and SHA-256. Protobuf serialization is never treated as canonical
research identity material.

## Trust and authorization boundaries

Descriptor-closure tests prove that Discovery and Research request/response
graphs cannot reach holdout contracts, generic internal job specifications,
locked sample windows, broad data-plane DTOs, or caller-selected backtest
specifications. They use a narrow `DevelopmentDatasetReference` dependency
leaf. This is a structural DTO guarantee only: Phase 4 and Phase 5 must resolve
opaque snapshot IDs under a server-owned role policy before persistence or
execution.

Holdout contracts bind a canonical period, one immutable freeze manifest, one
content-addressed complete evaluation plan, distinct attributable approvals,
and one irreversible grant lifetime. Grant consumption accepts no caller
factor, plan, budget, or backtest specification. The submitted timestamp must
satisfy `issued_at <= submitted_at < expires_at`. Phase 2 validates the
contract; Phase 3 and Phase 4 still own transactional persistence and runtime
capability enforcement.

Capabilities and secrets never appear in Protobuf application messages. They
belong to protected transport metadata and server interceptors. Provider DTOs
can still carry generic artifact references, so Phase 9 must enforce a
prompt-safe artifact namespace and deny the provider service identity access to
research and holdout storage.

Operational failures use non-OK gRPC status plus typed `ServiceError` details.
They cannot be represented as a `FactorRejection`, which remains a domain
outcome for a valid factor that was actually evaluated.

## Compatibility evidence

Current and baseline descriptor SHA-256:

```text
27a38398e290caee3fb857063c0f2adbbe43a7d2d44322103dbf4b72535be979
```

Five rejected pre-release descriptors are retained by their content SHA-256 in
`fixtures/contracts/protocol/history`. The history verifier pins the complete
inventory and demonstrates fail-closed behavior for a modified baseline,
modified history, and missing history. Because no earlier protocol was
released, current-versus-baseline is a seed trust anchor, not evidence of a
released-client migration.

The Python 3.14.4, Rust, and TypeScript wire producers emit byte-identical
`ProtocolInfo` fixtures with SHA-256:

```text
15687e24b490fe1051b5e642bc55607e64969680e035b2ccf1b6d89017ad9ccd
```

Compatibility coverage includes unknown-field tolerance, unknown enum and
`oneof` rejection at execution boundaries, source/baseline deterministic
regeneration, Buf lint and breaking checks, protocol feature negotiation,
artifact limits, typed failures, and cross-language semantic round trips.
Lossless relays must retain original immutable bytes; generated runtimes are
not assumed to preserve unknown fields after re-encoding.

Shared tabular negative vectors currently cover 105 job-record combinations,
59 holdout identity cases, 25 research-boundary cases, 14 holdout-boundary
cases, 18 artifact-reference cases, 15 protocol-negotiation cases,
8 protocol-selection negatives, 7 holdout job-binding cases, and 6 grant-lifetime cases. Canonical JSON fixtures add
positive, tamper, size-limit, ordering, and replay cases for factors, operator
semantics, holdout plans, and audit chains.

## Exit-gate evidence

The implementation was delivered in `ed85d2d`, followed by the contributor
language rule in `9ba4a36` and the CI cache correction in `0615d81`.
All are pushed to the refactor branch. The final host gates passed on
2026-09-07, and
[CI run 34101687394](https://github.com/hojiahao/loop_engine/actions/runs/34101687394)
passed all seven jobs for `0615d818e4e0e7fdf404db02ee9ca1c5e0683889`.

| Gate | Result |
| --- | --- |
| deterministic generation and protocol baseline guard | passed; checked-in bindings and wire producers match |
| Rust format, Clippy, unit, property, and integration tests | passed; 76 workspace tests |
| TypeScript format, lint, type, contract tests, and build | passed on host and CI |
| Python 3.14 protocol Ruff, mypy, contract tests, and build | passed; 236 protocol tests |
| cross-language wire and canonical fixtures | passed; no baseline change |
| host `just check` / `test` / `build` / `doctor` | all passed |
| research and isolated legacy regression | 1 research test; 216 legacy passed, 1 skipped, 12 NumPy warnings |
| clean DaoCloud development-container gate | passed in CI job `101677333027`; fresh Ubuntu 24.04 runner and unique Compose volumes |
| commit and remote branch push | implementation through `0615d81` pushed |
| GitHub Actions for pushed commit | run `34101687394`: 7/7 jobs passed |

The first run `34101082316` exposed a setup-uv cache-location mismatch after
successful Python test steps. The correction explicitly aligns the action
cache with `scripts/uv.sh`; no check was disabled. Its details are retained in
the Python 3.14 amendment verification record.

The subsequent phase-closure documentation commit changes evidence and status
only. It does not alter the verified schema, bindings, implementation, locks,
or test fixtures. Phase completion is not a merge to `main` or a product release.

## Explicitly deferred runtime work

Phase 2 deliberately does not claim any of the following:

- SQLite transactions, CAS revisions, leases, idempotency, or crash recovery
  from Phase 3;
- runtime holdout capability issuance and snapshot-role resolution from
  Phase 4/5;
- numerical operator conformance, factor evaluation, or stale-metric migration
  from Phase 4/6;
- portfolio accounting, execution, or returns from Phase 7;
- provider transport and prompt-artifact enforcement from Phase 9; or
- Run Harness authorization, sandboxing, budget enforcement, and recovery from
  Phase 10.

No market data was downloaded and no factor-performance, production-readiness,
or sample-outcome claim follows from this protocol phase.
