# ADR 0003: Repository layout and ownership boundaries

- Status: Accepted
- Date: 2026-09-04
- Owner: hojiahao

## Context

The legacy repository places orchestration, provider transport, numerical
research, configuration, and mutable checkpoints around one Python module tree.
That layout cannot enforce the service boundaries in ADR 0001 and makes it easy
for model-facing code to reach research or holdout state.

The migration must also preserve the Phase 0 regression baseline until the new
US-equities path can be reconciled. Moving legacy paths before replacement tests
exist would make failures harder to attribute.

## Decision

The target repository layout is:

```text
apps/
  web/                         React research and governance client
  providerd/                   TypeScript provider host
crates/
  loop-core/                   provider-neutral domain model and state machines
  loop-protocol/               generated Rust protocol bindings
  loopd/                       durable control plane and API
  loopctl/                     automation CLI
  loop-tui/                    SSH terminal client
python/
  loop_research/               primary data, factor, statistics, and backtest worker
  zipline_validation/          isolated independent event-driven replay
proto/loop/v1/                 shared provider-neutral wire DTOs
proto/loop/{role}/v1/          role-scoped gRPC service entry points
catalog/providers/             provider plugin descriptors
catalog/models/                model capability records
config/research/               universe, samples, costs, and research policy
config/data/                   data sources and entitlements
config/policy/                 capabilities, budgets, and approvals
migrations/postgres/           primary forward-only metadata migrations (ADR 0007)
migrations/sqlite/             historical migration evidence; no runtime backend
fixtures/contracts/            offline provider and protocol fixtures
fixtures/market/               synthetic and public numerical fixtures
tests/contracts/               cross-language compatibility tests
tests/integration/             service, concurrency, and recovery tests
tests/e2e/                     client and full-loop tests
infra/containers/              production container definitions
infra/compose/                 deployment composition
infra/observability/           OpenTelemetry configuration
docs/                          decisions, design, research, and operations evidence
legacy/a_share/                immutable legacy implementation and run evidence
```

Root-level files are limited to workspace manifests, lockfiles, stable command
entry points, development composition, contributor policy, and top-level
documentation. `.devcontainer/` remains at the conventional discovery path;
production container assets belong under `infra/containers/`.

## Dependency rules

1. `loop-core` and the shared `loop.v1` DTO package are provider-neutral and
   data-vendor-neutral. Role service packages may import shared DTOs, but shared
   DTOs never import a role service package.
2. `loopd` may depend on `loop-core` and protocol bindings. It must not import
   provider implementations or numerical research code.
3. `providerd` may depend on generated protocol bindings and provider plugins.
   It must not access research databases, snapshots, or holdout capabilities.
4. `loop_research` may depend on generated protocol bindings and data adapters.
   It must not route models or import provider plugins.
5. Web, TUI, and CLI clients call `loopd`; they never connect directly to
   provider, research, or persistence internals.
6. Large data crosses a process boundary only as an immutable reference with a
   URI, schema, row count, and SHA-256 digest.

The role packages are `loop.protocol.v1`, `loop.discovery.v1`,
`loop.provider.v1`, `loop.research.v1`, `loop.jobs.v1`, `loop.audit.v1`, and
`loop.holdout.v1`. They are separate authorization surfaces, not permission to
combine their implementations. `loopd` owns protocol negotiation, discovery
and research submission, durable jobs, audit, and holdout governance;
`providerd` implements the provider service; and `researchd` leases and executes
validated numerical job inputs without owning orchestration state. Long-running
research and backtest submission methods return a role-owned
`ResearchJobHandle` rather than executing numerical work in the RPC handler.
Discovery likewise owns a narrow development-reference input and returns a
`DiscoveryJobHandle`. Both handles contain only ID, projected status, revision,
and timestamps; neither role's request/response type graph can reach generic job
specifications, locked sample windows, holdout inputs, leases, or outcome bodies.
Opaque snapshot IDs still require server-owned role resolution and capability
checks before persistence or execution; the Phase 2 type graph does not claim
to prove their data role.
The durable enqueue, lease, and recovery behavior is implemented in Phase 3;
Phase 2 defines and tests only the protocol boundary.

These rules will be enforced by language dependency checks and integration
tests, not only documentation.

## Migration sequence

- Phases 1-12 build the target layout alongside the frozen legacy paths.
- Legacy regression tests remain mandatory while replacements are introduced.
- Phase 13 copies the legacy code, configuration, logs, and outputs into
  `legacy/a_share/`, verifies pre/post checksums, then updates test entry points.
- US-equities state starts empty and never imports A-share admission, direction,
  performance, or failure-memory state.
- The legacy paths are removed from active runtime resolution only after the
  checksum manifest and replacement regression suite pass.

## Consequences

The repository temporarily contains both layouts. This duplication is explicit
and bounded by Phase 13 rather than hidden behind ambiguous imports. Ownership
and test boundaries can be added incrementally without destroying baseline
evidence.
