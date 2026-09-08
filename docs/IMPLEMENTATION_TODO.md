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

## Phase 1 - Reproducible toolchain (`complete`)

- [x] Pin Rust, Node.js, pnpm, Python, uv, and container toolchains.
- [x] Create the Rust, TypeScript, and Python workspace layout.
- [x] Add `just bootstrap`, `just check`, `just test`, `just build`, and
      `just doctor` entry points.
- [x] Add formatting, linting, type checking, unit tests, and CI matrices.
- [x] Add a development container and deterministic bootstrap documentation.
- [x] Pass host and clean-container gates and record the evidence.
- [x] Commit and push the Phase 1 implementation to the remote branch.

## Phase 1 amendment - Python 3.14 uv workspace (`complete`)

- [x] Pin the primary Python toolchain and package constraints to 3.14.4.
- [x] Replace three independent uv projects with one root workspace, lockfile,
      and `.venv`.
- [x] Update local wrappers, CI, container isolation, and wire producer metadata.
- [x] Pass host and clean-container gates on the amended toolchain.
- [x] Commit, push, and record a successful remote CI run.

## Phase 2 - Core contracts (`complete`)

Implementation commit `0615d81` is pushed. Host `just check/test/build/doctor`
and all seven jobs in GitHub Actions run `34101687394` passed, including the
clean DaoCloud container gate. Detailed evidence is recorded in
`docs/verification/phase-02-core-contracts.md`. Completion does not imply a
merge to `main`, a production release, or implementation of later phases.

- [x] Define versioned Protobuf contracts for jobs, factors, data, models,
      streams, backtests, and audit events.
- [x] Define canonical `FactorSpec` and typed content blocks.
- [x] Generate Rust, TypeScript, and Python bindings.
- [x] Add schema evolution, unknown-field, and cross-language round-trip tests.
- [x] Add pure protocol negotiation and persisted-selection availability checks.
- [x] Pass final workspace and clean-container gates.
- [x] Commit, push, and record a successful remote CI run for phase closure.

## Phase 3 - Durable state (`in_progress`)

Implementation follows ADR 0006's transactional invariants and the owner's
2026-09-08 PostgreSQL amendment in ADR 0007. The published SQLite checkpoint is
historical evidence, not the current runtime backend. Production mutating RPCs
remain unavailable until authorization and reference resolution are implemented.
No Phase 3 completion is claimed by a storage checkpoint.

- [x] Add the original SQLite migrations, constraints, and indexes (historical).
- [x] Provision the isolated production PostgreSQL database and least-privilege
      identities; verify encrypted application login without opening public ports.
- [x] Port migrations, transactions, audit, role submissions, and leases to
      PostgreSQL; separate administrative DDL from runtime schema verification.
- [x] Pass the PostgreSQL TLS, migration, corruption, rollback, 2/4/8-process,
      cancellation, and kill/restart regression gates.
- [x] Implement revisions, leases, heartbeats, idempotency, and state transitions.
- [x] Add crash, cancellation, concurrent writer, and restart recovery tests.
- [x] Retain domain command interfaces independent of PostgreSQL SQL and pools.
      Immutable market data remains outside the metadata database.
- [x] Map four role-owned submission requests into transactional jobs, with
      protocol availability, immutable receipts, and atomic audit append.
- [x] Verify atomic sealed-period registration, immutable replay, default-deny
      access, canonical identity, and monotonic lifecycle constraints.
- [ ] Implement holdout approval, grant, and all-or-nothing batch consumption.
- [ ] Pass host, clean-container, and remote CI gates; commit and push evidence.

The storage/lifecycle checkpoint covers 2/4/8 independent OS writers and real
kill/restart at transaction and lease boundaries. These checked items do not
close the phase: holdout batch transactions and final gates are still required.
Production transport authentication and reference registries remain unavailable,
and fixture admission policies exist only in tests.
Evidence: `docs/verification/phase-03-durable-state.md`.

PostgreSQL/period-registration checkpoint `d85ae71` is pushed. All seven jobs in
GitHub Actions run `34200778090` passed, including 142 Rust tests, independent
process/fault cases, unified workspace commands, and the clean DaoCloud container.
The production Rust executable verified TLS/session settings and both installed
migration checksums. These results close this backend checkpoint, not Phase 3;
approval, grant issuance, and atomic batch consumption are the next work items.

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
- [ ] Prove every evaluator conforms to its resolved operator semantic contract
      with cross-language goldens, missing/constant-window properties, and
      reference numerical comparisons; bind source changes through
      `ResearchProvenance.source_code_sha256`.
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

- [ ] Refresh the early architecture diagram for the accepted PostgreSQL backend.
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
