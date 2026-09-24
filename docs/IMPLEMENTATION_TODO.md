# Loop Engine implementation checklist

This checklist is the execution record for the US-equities Loop Engineering
refactor. A phase may move to `complete` only after its code, tests,
documentation, commit, and remote push have passed the documented exit gate.

Status values: `pending`, `in_progress`, `blocked`, `complete`.

## Delivery order

The owner's 2026-09-22 instruction is to finish every Phase 9 delivery unit
before the first merge to `main`. Continue task-sized Chinese commits and pushes
on `refactor/us-equities-loop-runtime`; do not merge an intermediate Provider
checkpoint. After Phase 9 acceptance, prepare the product-oriented README and
reviewed pull request before merging. The README describes usable capabilities,
installation, configuration and workflows; phase progress and acceptance history
stay in the repository's maintenance documentation. Do not include that history
in product screens or production runtime images. Development/test containers may
still need source documentation for their verification commands. The final
release-documentation gate remains part of Phase 14.

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

## Phase 3 - Durable state (`complete`)

Implementation follows ADR 0006's transactional invariants and the owner's
2026-09-08 PostgreSQL amendment in ADR 0007. The published SQLite checkpoint is
historical evidence, not the current runtime backend. Production mutating RPCs
remain unavailable until authorization and reference resolution are implemented
in the later capability/data/runtime phases. Individual historical checkpoints
did not close Phase 3; the combined final evidence below does.

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
- [x] Implement immutable human approvals with authenticated attribution,
      bounded validity, canonical records, replay, and transactional audit.
- [x] Implement single-use grants, distinct-human policy resolution, expiry,
      revocation, and immutable approval attachment.
- [x] Implement all-or-nothing frozen-plan batch consumption and job insertion.
- [x] Pass host, clean-container, and remote CI gates; commit and push evidence.

The final storage/lifecycle implementation covers 2/4/8 independent OS writers,
real kill/restart at transaction and lease boundaries, and atomic full-plan
batch consumption. All Phase 3 exit gates have passed.
Production transport authentication and reference registries remain unavailable,
and fixture admission policies exist only in tests.
Evidence: `docs/verification/phase-03-durable-state.md`.

PostgreSQL/period-registration checkpoint `d85ae71` is pushed. All seven jobs in
GitHub Actions run `34200778090` passed, including 142 Rust tests, independent
process/fault cases, unified workspace commands, and the clean DaoCloud container.
The production Rust executable verified TLS/session settings and both installed
migration checksums. These results close this backend checkpoint, not Phase 3;
the later approval, grant issuance, and atomic batch checkpoints are separate.

The subsequent human-approval storage checkpoint adds 19 PostgreSQL tests plus
approval cases in the 2/4/8-process and kill/restart matrices. Local targeted
tests and `just check` pass. Commit `47e8128` is pushed and all seven jobs in
GitHub Actions run `34209334491` passed, including the clean-container gate.
ADR 0008 records the authority and identity boundaries. Production approval
policies still deny, migration 3 is not yet deployed to production, and grant
issuance and atomic batch consumption remain open. Phase 3 is not complete.

Grant lifecycle checkpoint `eb95170` is pushed under ADR 0009:
single-use issuance, independent approval attachment, verified reads, expiry,
revocation, immutable replay, and deferred aggregate constraints. The 26 grant
tests, 19 approval tests, 15 period tests, library fault tests, and expanded
2/4/8-process matrix pass. Full `just check` also passes. All seven jobs in GitHub
Actions run `34303072037` pass, including unified workspace and clean-container
gates. No production grant is issued.

Phase 3 closure: atomic batch implementation `e8572cf` and fixture lifecycle fix
`aa3bd2b` are pushed. GitHub Actions run `34311911291` passed all seven jobs,
including unified workspace and clean DaoCloud container gates. Final host
`just check/test/build/doctor` passed: Rust 207 plus two explicitly exercised
subprocess helpers, TypeScript 60, Python protocol 240, research 1, and legacy
216 passed / 1 skipped. Production migrations 1 through 5 are installed; the
actual Rust executable verified TLS, session settings and every checksum.
Production jobs, periods, grants and batches are empty. Fixture materializers
are not production research parsers, and no holdout capability was issued.

## Phase 4 - Research-integrity invariants (`complete`)

All five agreed foundation delivery units are published and remotely verified.
The final unit is `9c02629`; CI run `34747553685` passed all seven jobs, including
the clean DaoCloud container. This closes the shared integrity and authorized
raw-factor boundary. It does not claim a market-data adapter, portfolio backtest,
production holdout execution or autonomous discovery loop; those remain the
explicit later-phase deliverables below.

The owner-approved foundation delivery units were executed in this order, one
complete task per implementation commit. Tests and documentation are part of
that task, not additional delivery units. The same rule applies to later phases.

1. Durable Sharpe/perturbation/random state, recovery and failed-candidate wiring
   (`complete`: implementation `8095a9a`, fixture correction `5dabadd` and cache
   correction `62dbcb5` are pushed. Run `34465141418` passed all seven jobs).
2. Shared admission/readmission, coverage, trial/retirement accounting and overrides
   (`complete`: implementation `365875d` is pushed. Full local `just check/test`
   passed; run `34566128777` passed all seven jobs).
3. Trusted manifest resolution for calculation, result reads and exports
   (`complete`: implementation `f1b87a8` is pushed. Local
   `just check/test/build/doctor` and all seven jobs in run `34581890194` pass).
4. Runtime identity, holdout capability and actual data-access isolation
   (`complete`: implementation `e1931f5` is pushed. Local
   `just check/test/build/doctor/test-isolation` and all seven jobs in
   run `34695520155` pass. No production endpoint is enabled).
5. Canonical AST/operator/numerical integration into authorized execution
   (`complete`: `9c02629` is pushed. Local
   `just check/test/build/doctor/test-isolation` and all seven jobs in
   run `34747553685` pass).

- [x] Port all eight existing fixes into shared contracts and regression tests.
- [x] Add holdout capabilities, provenance invalidation, canonical SHA-256 IDs,
      missing-window skew tests, durable perturbation state, unified readmission,
      failed-hash filtering, and return-based PnL correlation.
- [x] Implement numerical reference primitives for actual-count skew, causal
      windows, valid NAV returns and return-derived correlation; verify against
      independent SciPy goldens and bounded Hypothesis properties.
- [x] Bind the numerical primitives to authorized canonical factor execution;
      enforce session alignment, coverage and versioned operator semantics.
- [x] Share original-run integrity and current-context freshness assessment
      across Rust, TypeScript and Python; reject unresolved and stale metrics.
- [x] Enforce provenance checks in durable result registration and current-result
      read/export paths, with trusted manifest resolution and restart coverage.
- [x] Commit and verify transactional backtest result registration and the
      current-result repository gate under ADR 0013. Production resolvers and
      audited exports remain separate required integrations.
- [x] Add audited current-metadata exports with fresh checks on every retry,
      transactional receipts, bounded deadlines and process/crash tests.
- [x] Implement the read-only NAV correlation CLI with strict observation-interval
      alignment, explicit cash-flow basis, byte provenance and subprocess tests.
      This diagnostic does not close authorized backtest execution integration.
- [x] Implement transactional development-backtest rejection memory in submission
      and acquisition, with exact frozen-context matching and 19 PostgreSQL
      regression tests. This is an internal command-path integration, not enabled
      production execution. Commit `70b44c5` is pushed and all seven jobs in
      run `34441752040` passed. Generalized factor-evaluation/readmission remains
      open (ADR 0015); perturbation integration is delivery unit 1 above.

ADR 0011 starts the numerical portion in the Python research package: actual
sample-count skew, stable central moments, causal windows, valid NAV returns and
return-derived correlation. Research-package isolation tests pass: 45 numerical
cases plus one health test. Full `just check` passes; SciPy 1.18.1 and Hypothesis
6.167.1 are installed under the root Python 3.14.4 workspace and pinned in
`uv.lock`. This checkpoint does not close Phase 4 or enable production evaluation.
Checkpoint `7e6352a` is pushed; all seven jobs in GitHub Actions run `34315225076`
passed, including the clean-container gate. A subsequent local full-suite run
exposed a migration namespace race; its follow-up is recorded in the Phase 3
verification document and is not counted as a passing local gate.

ADR 0012 defines immutable six-component provenance snapshots and distinguishes
recorded-run inconsistency from stale or unresolved current metrics. Shared
cross-language vectors and snapshot validation are implemented; trusted result
registration, current-result reads/exports and restart invalidation remain open.
Checkpoint `13aebad` is pushed. All seven jobs in GitHub Actions run `34324674436`
passed, including unified workspace and clean DaoCloud container gates.
Checkpoint evidence: `docs/verification/phase-04-research-integrity.md`.

Transactional result implementation `3e91027`, Clippy correction `275e190` and
bounded Rust-download correction `4581c53` are pushed. Run `34333550936` passed
all seven jobs, including unified and clean DaoCloud container gates. This
accepts ADR 0013's internal storage checkpoint, not production manifest resolution
or Phase 4 completion. The next checkpoint is the audited export gate in ADR 0014.

Audited metadata export checkpoint `214e170` is pushed. Run `34431407693`
passed all seven jobs, including complete Rust regressions, unified workspace
commands and the clean DaoCloud container. ADR 0014's internal repository gate
is accepted; it does not provide a user-facing file exporter or enable production
reference resolution. The remaining Phase 4 items above are unchanged.

The additive NAV diagnostic reuses ADR 0011's return kernel without changing
numerical semantics, protocol IDs or database state. The actual CLI consumes
bounded local CSV files, rejects shifted dates and emits explicit diagnostic
limitations. Local research tests are 95 passed (49 new), and `just check`
passes. Publication and remote acceptance are pending at this checkpoint's
commit time. Usage and rollback: `docs/development/nav-correlation.md`.

NAV diagnostic implementation `8a39cdc` is pushed. Run `34434276768` passed
all seven jobs, including the isolated research environment, unified workspace
and DaoCloud container gates. This accepts the read-only diagnostic step, not
authorized result export or production research execution.

## Phase 5 - US-equities data plane (`in_progress`)

Delivery units (one complete implementation/test/documentation commit
each, followed by push and remote acceptance):

1. Security master, historical ticker resolution and bitemporal market/fundamental
   records, with an executable local point-in-time query (`complete`, ADR 0021;
   `523a736` is pushed. Local `just check/test/build/doctor` and all seven jobs
   in CI run `34749353894` pass).
2. SEC and Alpaca development adapters, installed dependencies, bounded downloads,
   entitlement checks and explicit development limitations (`complete`, ADR 0022;
   `cc462da` is pushed and CI run `34765587180` passes all seven jobs. Local gates,
   live SEC capture and offline replay pass. The later 2026-09-14 operational
   acceptance verifies real Alpaca IEX access and replay; latest SIP access is
   explicitly denied. See the live acceptance record below).
3. Sharadar production adapter and optional WRDS/Databento paths, with credentials
   and entitlement gates; no unlicensed production claims (`complete`, ADR 0023;
   `32bafdb` is pushed and CI run `34804345277` passes all seven jobs. This closes
   the acquisition implementation, not licensed live access or PIT certification).
4. Immutable Parquet snapshots, quality/lineage/coverage checks and reproducible
   data synchronization through 2026-08-31 (`complete`, ADR 0024; `67854a5` is
   pushed and all seven jobs in CI run `34811496117` pass. Source snapshots remain
   private and are not automatically admitted factor panels).

- [x] Implement the bounded, offline XNYS session-date gate, pinned calendar
      dependency and actual NAV CLI integration. This is a Phase 4 alignment
      dependency, not completion of the data plane or a full calendar manifest.
- [x] Implement security master and bitemporal market/fundamental schemas with
      executable local historical queries. This does not certify vendor coverage;
      unit 1's local gates, commit, push and remote CI have passed.
- [x] Implement SEC plus Alpaca development adapters, with bounded acquisition,
      first-observed semantics, separate SIP probing and byte-verified cache replay.
      Unit 2 publication and remote CI are complete; data quality is development-only.
- [x] Implement Sharadar production adapter and optional WRDS/Databento adapters.
      Full host gates and publication pass; live vendor access remains unverified.
- [x] Build immutable Parquet snapshots, validation, lineage, and entitlement
      reports through 2026-08-31.
- [x] Implement and locally verify offline source-access preflight shared with
      synchronization, and document supplier credential onboarding (ADR 0025;
      `754486e` is pushed and all seven jobs in CI run `34816439735` pass).

All four coding delivery units have passed local and remote gates. Phase 5's
production-data exit remains open: actual licensed historical security/universe,
delisting and PIT coverage have not been attested. Alpaca and Nasdaq Data Link
credentials are now privately configured; the Sharadar subscription scope and
matching license declaration are still unconfirmed. Credentials alone do not
close that data-quality gate.

Owner's latest 2026-09-14 sequencing instruction: finish all remaining Phase 5
acceptance before beginning Phase 6. The offline handoff is published and remotely
verified. The current task records actual SEC/Alpaca acquisition, offline replay,
Parquet materialization and quality validation using the existing installed CLI.
It introduces no new service, source adapter or dependency.

Local live acceptance passes: six IEX bars for AAPL/MSFT on 2020-12-28 through
2020-12-30, eight SEC facts selected in 2020, and the complete source snapshot
`sha256:3846dc0bc1140095cd7c50026334bc752ff92a26e3eeff38d070b1574b0d8370`.
Each selected stock has three expected and three observed sessions; two current
asset records are correctly excluded from the historical selection. The snapshot
has 14 rows and remains `production_eligible=false`. Publication/CI of this live
acceptance task must be recorded before closing its delivery.

Remaining Phase 5 exit work, in order:

1. Confirm an actual licensed source's subscribed tables, historical scope,
   authorization dates and internal-research/local-storage rights. Current
   Sharadar preflight sees the key reference but returns `license_denied`.
2. Pin that real license declaration and perform a bounded supplier request,
   then verify its receipt and source Parquet offline. No invented declaration,
   subscription purchase or automatic increase in spending is permitted.
3. Expand authorized selections within explicit request/byte/time budgets and
   audit historical security/universe coverage, corporate actions, delisting
   returns and PIT/revision availability through 2026-08-31. Preserve missing
   evidence as failed/unverified checks; a subscription does not certify quality.
4. Publish the resulting acceptance evidence and pass the task's remote gates.
   Phase 6 remains pending until this exit is satisfied or the owner explicitly
   changes the sequencing/data requirements. Optional WRDS and Databento accounts
   are not both required if the selected source meets the agreed coverage.

The first data-plane step pins and installs `exchange-calendars==4.13.2` in
the existing Python 3.14.4 uv workspace. It adds explicit XNYS validation to the
read-only NAV diagnostic, with date-sequence digests and offline 2005-2026
generation. Research tests pass in ordinary and isolated environments: 123
passed, including 28 new calendar cases. `just check` passes. Remote acceptance
is pending at commit time. This advances a Phase 4 dependency without closing
Phase 4 or claiming licensed/PIT market data. Evidence and rollback:
`docs/verification/phase-05-us-data.md`.

Owner's subsequent 2026-09-14 decision defers paid subscriptions and authorizes
continued implementation using synthetic and public-development evidence. The
live acceptance task `e2a04de` is pushed; all seven jobs in run `34822845327`
pass. The licensed coverage exit above remains deferred, not passed. It no longer
blocks Phase 6 development; production data admission stays disabled.

## Phase 6 - Factor engine (`complete`)

All three delivery units are published. Final implementation `b792601` passes
all seven jobs in CI run `35058329373`, including complete workspace and clean
DaoCloud container gates. The numerical/authorized trial workflow is closed;
licensed historical data and formal portfolio accounting remain separate gates.

Delivery units (implementation, tests and documentation together; publish and
verify each before the next):

1. Build causal factor panels from verified data and explicit security histories,
   with complete calendar axes, eligibility and knowledge-time checks; connect
   the output to the existing authorized factor execution path (`complete`:
   `39959d7` is pushed; all seven jobs in CI run `34924328910` pass). ADR 0026 and
   `docs/verification/phase-06-factor-engine.md` record 49 new panel/source cases,
   693 passing research tests and the actual TLS/PostgreSQL/Python handoff.
   No paid source, protected sample or production-quality claim is included.
2. Add versioned cross-sectional winsorization, standardization and industry/
   size/beta neutralization, with deterministic missing/rank-deficient behavior
   and independent numerical goldens (`complete`: `e881d3a` and `a1ec5e3` are
   pushed; all seven jobs in CI run `34947453231` pass). ADR 0027 binds versioned
   panels, exposures and results to frozen policies; raw inputs remain compatible.
3. Integrate the complete factor-research evaluation and trial/admission workflow,
   with frozen preprocessing/operator provenance, coverage, duplicate/failure
   filtering, determinism and no-lookahead acceptance (`complete`; ADR 0028).
   `b792601` is pushed; all seven jobs in CI run `35058329373` pass, including
   independent 2/4/8 writers and kill/restart. Evidence is in the Phase 6 record.
   Passing numerical coverage means ready for later backtesting, not admission
   or portfolio performance. Production backtesting remains Phase 7.

- [x] Port and specify operators, canonical AST evaluation, neutralization,
      coverage checks, and trial registry.
- [x] Prove every evaluator conforms to its resolved operator semantic contract
      with cross-language goldens, missing/constant-window properties, and
      reference numerical comparisons; bind source changes through
      `ResearchProvenance.source_code_sha256`.
- [x] Add golden, property, determinism, and look-ahead tests.

## Phase 7 - Primary backtester (`complete`)

All five delivery units below are implemented, committed, pushed and accepted.
Final global-statistics implementation `e325368` passes all seven jobs in
exact-commit CI `35569521771`, including independent process/crash matrices,
unified workspace and clean DaoCloud container gates. Phase 7 is complete.
This includes the authenticated cross-trial statistics extension requested on
2026-09-20. Licensed-data full retesting remains Phase 5/13; completed semantic
and economic acceptance remain later gates. No subscription is authorized and
development/synthetic evidence does not establish investment performance.

Delivery units, each with an executable workflow, negative-path tests, numerical
goldens, documentation, a Simplified Chinese commit and remote acceptance:

1. Frozen next-session portfolio replay and cash/NAV ledger (`complete`; ADR 0029): consume
   verified factor values and explicit raw execution observations; implement
   deterministic long-only ranking, sizing, commissions/spread assumptions,
   orders/fills, holdings, cash, NAV and simple returns. Add bounded administrative
   CLI execution/replay and immutable receipts. Unsupported actions/shorting fail
   explicitly; synthetic/development evidence cannot claim production eligibility.
   Local final acceptance: 77 affected tests, Ruff, strict mypy, Rust formatting
   and workspace Clippy pass. Commit `cf2750d` is pushed; all seven jobs in CI
   `35065117368` pass. See `docs/verification/phase-07-primary-backtest.md`.
2. Market/accounting completeness (`complete`; ADR 0030): integrate PIT execution inputs,
   splits, dividends, delisting settlements, short availability, borrow costs,
   participation/capacity and price-impact policy. Exercise corporate-action,
   suspended/untradable-security, financing and insolvency golden ledgers.
   The versioned development profile also consumes dated SEC/TAF pass-through
   assumptions. Unsupported complex actions or unresolved intraday recalls fail
   closed. It does not certify a broker account, licensed coverage or production
   eligibility. Local acceptance: 120 affected regressions, followed by 46 final
   market cases after the naming refactor; Ruff/format and strict mypy pass.
   Commit `4953dc9` is pushed; CI `35082001411` passed all seven jobs, including
   unified workspace and the clean DaoCloud container.
3. Statistical evaluation (`complete`; ADR 0031): IC/Rank IC, grouped performance, turnover,
   drawdown, risk exposures and uncertainty; bind multiple-testing procedures
   and trial counts to complete experiment evidence. Insufficient inputs produce
   explicit unavailable results, not invented statistics or admissibility.
   The opt-in frozen profile adds administrative statistics run/replay, complete
   declared-family BY-FDR, DSR and exhaustive CSCV. Authenticated global trial
   completeness remains part of unit 4. Local combined regression: 187 passed,
   one native-environment verification timeout; the affected case passes alone
   in 6.42 seconds. All 65 new cases pass. Ruff, strict mypy, naming and Rust
   formatting pass. Commit `a512220` is pushed; all seven jobs in exact-commit
   CI run `35180788482` pass.
4. Authorized execution and phase acceptance (`complete`; ADR 0032): connect the installed
   producer to frozen contracts, runtime identity/data capability, lease fencing,
   durable results, current reads/exports and shared admission. Cover protected
   execution boundaries, independent processes, interrupted runs and deterministic
   replay; preserve the separate Phase 8 independent-validation requirement.
   Implementation now includes the pinned mTLS producer, verified numerical
   predecessor, complete database-local job/attempt commitment, conservative
   global BY bound, atomic results, current reads/audited exports and the shared
   admission prerequisite. Global DSR/PBO and Phase 8 independent reconciliation
   remain explicitly unavailable. All 26 affected Python cases have passing
   evidence across a combined run and one deadline-related isolated rerun;
   local Clippy, TypeScript and naming gates pass. Commit `4addeb8` is pushed.
   CI `35196842306` passes six jobs, including all corrected Rust/process cases
   in the unified workspace and clean DaoCloud container. The standalone Rust
   job hit its 20-minute total limit: the complete Rust tests take 21m33s before
   build/setup overhead. Its bounded budget is now 35 minutes; production
   deadlines are unchanged. The corrected exact-commit run `35202271352` passes
   all seven jobs; Rust completes in 24m57s, workspace in 32m46s, container in 33m7s.

- [x] Integrate portfolio/NAV generation with the Phase 4 authorization and
      numerical integrity gates; raw factor values and imported synthetic
      results must never be presented as a completed portfolio backtest.
- [x] Implement next-tradable-time portfolios, costs, borrow, turnover,
      capacity, risk exposures, IC analytics, and multiple-testing controls.
- [x] Validate supported accounting paths against synthetic golden ledgers;
      unsupported market events fail explicitly as described in ADR 0030.

5. Global statistical evidence (`complete`; ADR 0036): capture the complete
   authorized database trial population, resolve registered portfolio lineage,
   build an exactly synchronous strategy-return matrix, and reuse the existing
   DSR/CSCV kernels. Register and reread the report through existing job/lease/
   receipt/audit transactions. Retain incomplete, failed, incompatible and retry
   evidence explicitly; do not select only successful trials or call job counts
   independent strategies. Include numerical, authority, drift and replay gates.
   This is one implementation/test/documentation delivery unit, followed by its
   Chinese commit, push and exact-commit CI. Local acceptance now includes six
   actual mTLS/PostgreSQL cases, 78 combined numerical regressions, an expanded
   eight-case worker suite, 306 protocol cases and TypeScript/protocol gates.
   Commit `e325368` is pushed; exact-commit CI `35569521771` passes all seven
   jobs, including full Rust/process/crash, unified workspace and clean DaoCloud
   container gates. This closes the development primary/statistical scope;
   licensed data and economic/semantic acceptance are not waived. See
   `docs/verification/phase-07-global-statistics.md`.

## Phase 8 - Independent validation (`complete`)

Final implementation `8f30526` and bounded CI correction `989584b` are pushed.
Exact-commit run `35686353676` passes all seven jobs: Rust 55m08s, unified
workspace 69m48s and clean DaoCloud container 73m25s. All four development
delivery units below are accepted. Licensed production data, preregistered
economic thresholds and completed semantic review remain separate gates.

Delivery units (implementation, tests, documentation, Chinese commit and push
together; finish one before starting the next):

1. Alphalens statistics (`complete`; ADR 0033): isolated locked dependency,
   verified raw-input export, actual IC/group/turnover calculation, explicit
   differences, immutable reports and read-only replay. Confirm Python 3.14.4
   compatibility without downgrading the primary pandas 3 environment.
   Local acceptance: 37 independent-validator tests, 23 affected primary
   regressions, full `just check` and the isolated package build pass. Commit
   `3be45a1` is pushed; exact-commit CI `35306864434` passes all seven jobs.
   Evidence and rollback are in
   `docs/verification/phase-08-independent-validation.md`.
2. Zipline accounting (`complete`; ADR 0034): implementation `ce4e246` and the
   clean-container bootstrap correction `7258607` are pushed. Exact-commit CI
   `35319727700` passes all seven jobs after retrying a DaoCloud registry 503.
   A separately locked
   Python 3.12.13 process uses actual Zipline 3.1.1 blotter/ledger components,
   independently computes both frozen execution profiles, compares all seven
   ledgers and preserves native/economic NAV bridges and immutable replay.
   The primary Python 3.14.4 environment is unchanged. Actual primary-export/
   independent subprocess tests pass for both profiles; all 57 affected primary
   regressions and 46 independent tests pass, as do the isolated build and local
   style/type/lock gates.
3. Authorized reconciliation (`complete`; ADR 0035): bind both independent receipts to
   actual registered primary evidence and frozen comparison policy, integrate
   current reads/replay and shared admission, and test role/lease/staleness,
   mismatch, interruption and rollback paths. Independent numerical agreement
   cannot waive licensed-data quality, semantic review or other admission gates.
   Implementation `eaa638a` is pushed. Local check/build/doctor pass; the first CI
   passes six of seven jobs, including full Rust/2/4/8-process and unified
   workspace gates, but exposes container-specific test interpreter/cache paths.
   Corrections `ca7b26d` and `cf4cc09` retain all tests and production bounds.
   Exact-commit CI `35498968066` passes all seven jobs, including the clean
   DaoCloud container. The real local acceptance/restart/admission regression
   also passes. Unit 3 is accepted.
4. Global statistical evidence binding (`complete`, ADR 0037; depends on Phase 7 unit 5):
   bind the registered full-population report to each independently reconciled
   candidate and the shared admission command. Missing, stale or unavailable
   statistical evidence must remain explicit. Preserve existing immutable
   reports and the separate licensed-data/semantic-review prerequisites. Deliver
   implementation, negative-path tests, documentation and one Chinese commit
   before moving to the Provider platform.
   V2 policy/report binding, whole-population transaction rechecks and explicit
   pending/stale/unavailable gates are implemented locally. All six new real
   mTLS/PostgreSQL/Alphalens/Zipline cases and 26 affected Python workflows pass.
   The full v1 Rust compatibility/restart/admission case and final all-targets
   Clippy also pass. Implementation `8f30526` is pushed. Run `35587880648` passes
   six jobs, including full workspace and clean-container gates; the standalone
   Rust job reaches its 60-minute total limit while tests are still passing.
   A workflow-only correction gives that job 75 minutes without removing tests
   or changing application deadlines. Corrected exact-commit run `35686353676`
   passes all seven jobs; see
   `docs/verification/phase-08-statistical-binding.md`.

## Phase 9 - Provider platform (`in_progress`)

Delivery units (each includes an executable workflow, negative-path contract
tests, usage/recovery documentation, a Chinese commit, push and remote gates):

1. Native OpenAI/Anthropic invocation (`complete`): connect the existing ProviderService to
   OpenAI Responses/Chat and Anthropic Messages through isolated TypeScript
   plugins; authenticate callers independently of Actor metadata, pin model and
   request policy, bound requests/cost/time, and return typed redacted failures.
   Exercise actual TLS/gRPC and local HTTP vendor fixtures. Unsupported content
   and operations fail explicitly; offline evidence is not live verification.
   Local native contracts now pass 52 tests, including the compiled executable,
   real TLS/gRPC, 2/4/8 independent journal writers, kill/restart, cancellation,
   clock regression and budget failures. Full `just check`, 115 TypeScript
   protocol tests and clean TypeScript check/test/build pass. Commit `fc00680` is
   pushed; all seven jobs in exact-commit CI run `35701264148` passed. See ADR 0038 and
   `docs/verification/phase-09-native-providers.md`.
2. Streaming and rich messages (`complete`): connect ordered stream events, tool calls/results,
   structured output, reasoning continuation, prompt caching and prompt-safe
   artifacts where each native protocol supports them. Test cancellation,
   truncated streams, interleaved blocks, malformed output and namespace denial.
   Native content/stream adapters, registered schemas, actor-private artifacts,
   reasoning-state recovery and cache-write accounting are implemented. The
   Provider suite passes 127 local tests with actual TLS/gRPC fixtures. Full
   `just check`, TypeScript test/build, 310 Python protocol and 27 Rust protocol
   tests pass. Chinese task commit `daae4d0` is pushed; all seven jobs in
   exact-commit CI run `35819112668` passed;
   see ADR 0039 and `docs/verification/phase-09-native-content.md`.
3. Additional native protocols (`complete`): implement Google GenerateContent/Interactions
   and Cohere V2 Chat as independent plugins with executable contract matrices,
   explicit capability differences and provider-specific errors/usage.
   All three routes now share the authenticated invocation, stream, schema,
   artifact and continuation paths. Native tool/JSON/media handling and explicit
   input-ceiling reservations are implemented; 60 additional contract cases cover
   the new protocols. Local `just check`, 187 Provider tests, 119 TypeScript
   protocol tests and TypeScript workspace build pass. Chinese task commit
   `d5d49de` is pushed; all seven jobs in exact-commit CI `35830578199` passed.
   See ADR 0040 and
   `docs/verification/phase-09-google-cohere.md`.
4. Cloud deployments (`complete`): implement AWS Bedrock Converse/ConverseStream, Azure
   OpenAI and Google Vertex AI authentication, region/deployment/model mapping
   and native parameter translation; verify with signed-request fixtures.
   Four routes share the existing authenticated invocation/journal path. Cloud
   identity, conservative input/Guardrail cost reservations, native rich messages,
   AWS binary framing, signed continuation and cancellation are implemented.
   Local `just check`, 269 Provider tests (82 new cloud cases), 119 TypeScript
   protocol tests and TypeScript workspace build pass. Chinese task commit
   `5594ea8` is pushed; all seven jobs in exact-commit CI `35842723030` passed.
   Evidence is in ADR 0041 and
   `docs/verification/phase-09-cloud-deployments.md`. This unit does not close
   Phase 9, attest cloud entitlement or permit a merge to `main`.
5. First-class vendor plugins (`complete`): Mistral, DeepSeek, Qwen/DashScope, xAI, Groq,
   Together, Fireworks, Cerebras, Perplexity, GLM/Zhipu, Kimi/Moonshot and MiniMax.
   Share a wire codec only when the official protocol permits it; preserve each
   vendor's authentication, parameters, capabilities, errors and usage.
   All twelve routes now execute through the authenticated service and existing
   journal, budget, schema and continuation boundaries. The 153 new local cases
   pass; the combined run passed 119 protocol and 421/422 Provider cases, with
   the unchanged CLI startup timeout passing all three isolated retest cases
   after disk cleanup. `just check` and TypeScript workspace build pass.
   Chinese task commit `e1055c6` is pushed; all seven jobs in exact-commit CI
   `35953153780` passed.
   See ADR 0042 and
   `docs/verification/phase-09-vendor-plugins.md` for evidence and limitations.
6. Compatible/self-hosted/gateway paths (`complete`): OpenAI/Anthropic-compatible endpoints,
   Ollama, vLLM, SGLang, llama.cpp, LM Studio, NVIDIA NIM, LiteLLM, Portkey and
   OpenRouter. Distinguish gateway identity from the upstream supplier and deny
   unproven capabilities. Administrative endpoint configuration is not model input.
   Twelve wire routes now use the existing authenticated service, budget and
   journal. All 172 new local cases pass, including private continuation recovery,
   gateway routing controls, contradictory receipts, cancellation and model
   substitution denial. Complete TypeScript regression passes 594 Provider and
   119 protocol cases; `just check` and workspace build pass. Chinese task commit
   `9d197c7` is pushed; all seven jobs in exact-commit CI `35958808670` passed.
   See ADR 0043 and
   `docs/verification/phase-09-compatible-routes.md`.
7. Model catalog (`in_progress`): merge versioned built-ins, official discovery, verified remote
   catalogs and administrative overrides; atomically reload validated snapshots
   without changing a running model resolution. Track implemented/contract/live
   verification separately from unavailability, deprecation and retirement.
   Implemented bounded native/list discovery, pinned Ed25519 catalogs, strict
   precedence, immutable publication and atomic SIGHUP activation with retained
   model/price pins. Targeted HTTP, TLS/gRPC, 2/4/8-process and compiled CLI
   workflows pass. Full regression passes 676 Provider cases (82 new) and 119
   protocol cases; `just check` and workspace build pass. Commit/push and
   exact-commit CI remain required. See ADR 0044 and
   `docs/verification/phase-09-model-catalog.md`.
8. Platform acceptance: verify bounded rate limits/retries and cost accounting,
   actual provider process/data isolation, dependency boundaries and the combined
   protocol matrix. Live smoke tests require credentials and explicit budgets;
   absent credentials remain an honest verification limitation, never a fake pass.

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
- [ ] Connect preregistered deterministic economic acceptance criteria and
      completed semantic review to the existing shared admission prerequisites.
      Statistical availability or independent agreement alone is not acceptance;
      licensed-data eligibility remains a separate Phase 5/13 gate.

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

- [ ] Rewrite the final README as product documentation: purpose, requirements,
      installation/deployment, configuration, workflows and examples, interfaces,
      operations, security, and data/provider limitations. Keep phase progress,
      commit histories, and troubleshooting investigations in `docs/`, not the
      product README. Document only verified release behavior and runnable
      commands, with explicit credentials, licensing and cost prerequisites.
      This is a release gate, not a claim that the product is already complete.
- [ ] Complete branding, operations documentation, SBOM, security scans,
      migration notes, release verification, pull request, and release tag.
