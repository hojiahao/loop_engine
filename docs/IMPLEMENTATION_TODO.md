# Loop Engine implementation checklist

This checklist is the execution record for the US-equities Loop Engineering
refactor. A phase may move to `complete` only after its code, tests,
documentation, commit, and remote push have passed the documented exit gate.

Status values: `pending`, `in_progress`, `blocked`, `complete`.

## Global invariants

- [ ] The Loop Runtime has no provider-specific branches.
- [ ] Holdout data is inaccessible to discovery and model tools.
- [ ] Every metric identifies its code, configuration, data, calendar, and
      environment provenance.
- [ ] No result is described as production-grade without licensed,
      survivorship-aware, point-in-time data.
- [ ] No secret is stored in Git, logs, checkpoints, or model prompts.
- [ ] Existing Git authorship and legally required attribution are preserved.

## Phase 0 - Baseline freeze (`complete`)

- [x] Confirm `main` is synchronized with `origin/main` at `f3fd8bf`.
- [x] Run the legacy test suite: 216 passed, 1 skipped.
- [x] Create and push the annotated tag `legacy-a-share-v0.1`.
- [x] Create branch `refactor/us-equities-loop-runtime`.
- [x] Record the Git tree, artifact hashes, dependency set, and stale metrics.
- [x] Record the architecture decision for Loop Engineering and Run Harness.
- [x] Record source-code and market-data licensing constraints.
- [x] Record the initial security and research-integrity threat model.
- [x] Re-run tests and validate all Phase 0 documents.
- [x] Commit and push the Phase 0 deliverables.

## Phase 1 - Reproducible toolchain (`pending`)

- [ ] Pin Rust, Node.js, pnpm, Python, uv, and container toolchains.
- [ ] Create the Rust, TypeScript, and Python workspace layout.
- [ ] Add `just bootstrap`, `just check`, and `just test` entry points.
- [ ] Add formatting, linting, type checking, unit tests, and CI matrices.
- [ ] Add a development container and deterministic bootstrap documentation.

## Phase 2 - Core contracts (`pending`)

- [ ] Define versioned Protobuf contracts for jobs, factors, data, models,
      streams, backtests, and audit events.
- [ ] Define canonical `FactorSpec` and typed content blocks.
- [ ] Generate Rust, TypeScript, and Python bindings.
- [ ] Add schema evolution, unknown-field, and cross-language round-trip tests.

## Phase 3 - Durable state (`pending`)

- [ ] Add SQLite migrations, WAL configuration, constraints, and indexes.
- [ ] Implement revisions, leases, heartbeats, idempotency, and state transitions.
- [ ] Add crash, cancellation, concurrent writer, and restart recovery tests.
- [ ] Define a future-compatible storage interface for PostgreSQL/object stores.

## Phase 4 - Research-integrity invariants (`pending`)

- [ ] Port all eight existing fixes into shared contracts and regression tests.
- [ ] Add holdout capabilities, provenance invalidation, canonical SHA-256 IDs,
      missing-window skew tests, durable perturbation state, unified readmission,
      failed-hash filtering, and return-based PnL correlation.

## Phase 5 - US-equities data plane (`pending`)

- [ ] Implement security master and bitemporal market/fundamental schemas.
- [ ] Implement SEC plus Alpaca development adapters.
- [ ] Implement Sharadar production adapter and optional WRDS/Databento adapters.
- [ ] Build immutable Parquet snapshots, validation, lineage, and entitlement
      reports through 2026-08-31.

## Phase 6 - Factor engine (`pending`)

- [ ] Port and specify operators, canonical AST evaluation, neutralization,
      coverage checks, and trial registry.
- [ ] Add golden, property, determinism, and look-ahead tests.

## Phase 7 - Primary backtester (`pending`)

- [ ] Implement next-tradable-time portfolios, costs, borrow, turnover,
      capacity, risk exposures, IC analytics, and multiple-testing controls.
- [ ] Validate every accounting path against synthetic golden ledgers.

## Phase 8 - Independent validation (`pending`)

- [ ] Install Alphalens Reloaded in the research environment.
- [ ] Isolate Zipline Reloaded in a compatible locked environment.
- [ ] Reconcile factor statistics, positions, trades, costs, and returns.

## Phase 9 - Provider platform (`pending`)

- [ ] Implement native OpenAI, Anthropic, Gemini, Bedrock, and Cohere codecs.
- [ ] Implement cloud deployment adapters and first-class vendor plugins.
- [ ] Implement compatible, self-hosted, and gateway transports.
- [ ] Implement the hot-reload capability catalog and provider contract suite.

## Phase 10 - Run Harness (`pending`)

- [ ] Implement typed context, tools, capability authorization, budgets,
      cancellation, retries, recovery, redaction, and structured outputs.

## Phase 11 - Loop Runtime (`pending`)

- [ ] Implement persistent discovery, maker/checker validation, feedback,
      perturbation, failure memory, bounded termination, and human escalation.

## Phase 12 - Web and TUI (`pending`)

- [ ] Implement the operational React application and Ratatui interface.
- [ ] Verify desktop/mobile layout, keyboard access, screenshots, and TUI flows.

## Phase 13 - Migration and retest (`pending`)

- [ ] Quarantine the A-share archive without rewriting audit evidence.
- [ ] Import semantically mappable expressions as non-admitted diagnostics.
- [ ] Start a clean US factor library, run discovery, freeze, and reconcile.
- [ ] Unlock 2021-2024 and 2025-01-01 through 2026-08-31 exactly once.

## Phase 14 - Release (`pending`)

- [ ] Complete branding, operations documentation, SBOM, security scans,
      migration notes, release verification, pull request, and release tag.
