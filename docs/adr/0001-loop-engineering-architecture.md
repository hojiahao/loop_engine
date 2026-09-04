# ADR 0001: Loop Engineering as the top-level architecture

- Status: accepted
- Date: 2026-09-04
- Decision owner: hojiahao

## Context

The legacy application is a Python process that combines candidate generation,
model calls, factor evaluation, filtering, and checkpoint persistence. The US
equities system needs durable multi-run orchestration, strict sample access,
provider portability, crash recovery, and independently reproducible research.

Harness Engineering was publicly named before Loop Engineering in 2026. The
terms do not have a universally standardized containment relationship. This ADR
therefore records the bounded definitions used by Loop Engine.

## Decision

Loop Engineering is the product-level architecture. It owns task discovery,
repeated execution, cross-run state, verification, feedback, budgets, stopping
conditions, and human escalation.

The Run Harness owns one agent execution: model transport, context, typed tools,
capabilities, sandboxing, cancellation, recovery, budgets, and telemetry.

The system is split by engineering responsibility:

- Rust `loopd` implements the durable Loop Runtime and Run Harness control plane.
- TypeScript `providerd` implements model protocols and provider plugins.
- Python `researchd` implements data, numerical factor evaluation, and backtests.
- React and Ratatui clients use the same versioned `loopd` API.

Control messages use versioned Protobuf/gRPC. Large tables remain immutable
Parquet artifacts; RPC messages carry only validated references and hashes.

## Enforced boundaries

- Loop and research packages cannot import provider implementations.
- Provider plugins cannot access research state or holdout storage directly.
- Model tools receive explicit capabilities; the holdout capability is never
  issued to discovery, review, UI, or normal research runs.
- Every state transition is durable, validated, idempotent, and auditable.
- A plugin or model catalog update cannot alter an already-started run.

## Consequences

The system has more explicit contracts and processes than the legacy script,
but failures become isolatable and reproducible. Adding a vendor changes a
plugin and capability description rather than the Loop Runtime. Python remains
the numerical research language while Rust owns reliability-critical control.
