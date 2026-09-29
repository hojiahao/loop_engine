# Phase 10 unit 3: Controlled tools and persistent context

Status: implementation, targeted acceptance and full local quality gates pass.
Publication CI `36549762127` exposed exhausted disposable PostgreSQL storage;
the capacity correction and full-suite rerun remain the closing gate. The
preceding unit's CI passed all seven jobs. No synthetic fixture establishes live
supplier verification.

## Requirement and design

Execute a real bounded conversation through the existing authenticated Provider
and development-data boundary. Persist both paid calls and the intervening
verified tool result. Reconstruct context from immutable receipts, enforce
cumulative job ceilings and deny unknown tools, altered history, protected data
and conflicting replay. Keep prior one-step behavior and migration history.

ADR 0048 records the scope and tradeoffs. The first tool is descriptive and
read-only; it does not evaluate or admit a factor. Migration 0012 adds ordinal
identity and tool evidence without rewriting historical requests or budgets.

## Required evidence

- Provider-compatible request digest goldens for the first and final typed turns;
  keep the original one-call golden unchanged.
- Real file validation and bounded safe tool output, with unknown names,
  arguments, schema/hash drift, protected samples and oversized context denied.
- Real PostgreSQL/mTLS Provider workflow: two supplier calls, one saved tool
  result, one final candidate, exact persisted assistant/result pairing.
- Restart after the first receipt, tool publication and final dispatch; completed
  model calls are never invoked again and committed context is reused verbatim.
- Cumulative token/USD/step bounds; wrong ordinal, stale lease/revision,
  conflicting result and corrupted history deny mutation.
- Independent 2/4/8 OS writers and crash cuts around tool and next-call commits.
- Previous one-call regressions, migration compatibility, full quality checks,
  a Simplified Chinese task commit, push and exact-commit remote CI.
- Real restricted PostgreSQL role using the deployment bundle's grants: model
  state transitions have UPDATE authority; immutable tool evidence does not.

Observed local evidence:

- The selected Rust suite passes **132 tests**, with zero failures and one ignored
  helper entry point invoked by its parent OS-process tests. This includes 20
  actual Discovery/Provider workflows, independent 2/4/8 writers, crash cuts,
  migration preservation, restricted deployment privileges and negative paths.
- Five Provider digest/schema cases pass, retaining the original single-call
  golden and matching both new typed conversation turns across Rust/TypeScript.
- Provider compilation and full `just check` pass: Rust formatting and Clippy
  with warnings denied, TypeScript format/lint/type checks, Python Ruff/mypy,
  naming rules, protocol generation and cross-language compatibility. The
  changed Provider fixture's format/lint checks also pass. Exact-commit remote
  CI remains required before closing the delivery unit.

No paid model, production database, licensed market-data download or real
holdout unlock is part of these synthetic acceptance cases. Disposable
PostgreSQL data and the temporary test runner were removed after the suite.

## Full-suite storage correction

Commit `b33f0f5` passed the four Python/TypeScript CI jobs. Rust, unified workspace
and clean-container jobs reached PostgreSQL errors `53100` (no space left), then
`57P03` during recovery. Their 1 GiB tmpfs volumes measured 1,037,844 KiB,
1,042,204 KiB and 1,048,576 KiB used respectively. This is an infrastructure
failure; the local targeted suite was insufficient evidence for full-suite
capacity. Rust formatting and Clippy had passed in those jobs.

The test-only entry point now requests 128 MB maximum/32 MB minimum WAL targets
and `pglz` full-page compression. The volume and memory ceilings remain fixed;
durability settings and test deadlines are not weakened. Existing schema history
must remain available to reopen/crash tests until fixture teardown. Managed gates
now report data and WAL usage separately. The real rebuilt fixture starts
successfully and reports the intended 128/32 MB targets with `pglz`, `fsync=on`,
`full_page_writes=on`, `synchronous_commit=on`, `wal_level=replica` and the
unchanged 300-second/0.9 checkpoint settings. Shell syntax, function-name checks
and whitespace checks pass. Startup configuration verification is not a
full-suite capacity measurement; the new exact-commit CI remains required.
An additional real PostgreSQL pressure probe created and retained 500 independent
schemas, applying all twelve current migrations in separate transactions. It
completed without storage failure: `pg_database_size` reported 518,092,467 bytes
and WAL files 134,217,728 bytes. WAL stayed at 128 MiB from schema 140 through 500,
with no manual checkpoint or intervening schema deletion. This verifies migration
pressure, not the complete behavioral suite or production capacity. The probe
and disposable database are removed afterwards.
Rollback restores the test
settings and recreates only the disposable service; production configuration,
research records and migrations are unaffected.

## Reproduction

```bash
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
./scripts/pnpm.sh --filter @loop-engine/providerd build
./scripts/pnpm.sh --filter @loop-engine/providerd exec vitest run test/harness-digest.test.ts --maxWorkers=1 --no-file-parallelism
bash scripts/postgres-test.sh start
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 ./scripts/cargo.sh test --config profile.test.package.loopd.codegen-units=256 --locked --offline -p loopd --lib -- runtime::discovery store::model_step runtime::model_codec runtime::service::discovery --test-threads=1
bash scripts/postgres-test.sh usage
bash scripts/postgres-test.sh stop
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CI=true just check
```

The local host uses the debug-symbol overrides and smaller `loopd` codegen units
above to reduce compiler memory pressure. Test selection, runtime deadlines,
checks and warning policy are unchanged; remote CI retains the workspace's
normal build profiles.

This is a closed two-call profile, not arbitrary model-selected tools or an
autonomous outer Loop. Context is bounded and immutable; this unit does not add
summarization, reasoning-state continuation, post-deadline reconciliation or
account-wide budgets. Provider-specific schema and model restrictions still
apply. No synthetic HTTP supplier result establishes live supplier verification.

## Rollback

Disable v2 plan execution on a migration-aware deployment. Preserve model steps,
tool results, plan objects, Provider journal and audit; no destructive down
migration or uncertain budget reset is supported. Test containers and temporary
fixtures are disposable and cleaned independently of production artifacts.
