# Development Backtest Failure Memory

The Rust store now consumes committed deterministic rejections automatically
in `JobRepository::submit`, `submit_role` and new lease acquisition. Callers do
not supply `failed_hashes`. `PreviouslyRejected` means skip redundant work, not
retry a database/worker fault. This is an internal, tested storage path; no
production RPC, factor worker or provider route is enabled by this change.

## Matching Rules

Only the exact frozen context matches. `context_sha256` is SHA-256 of
`ASCII("loop.backtest-rejection-context/v1") || 0x00 || compact_json`, with
these fields in this order:

1. `factor_spec_id`: validated canonical FactorSpec ID string.
2. `snapshot_ids`: validated sorted unique development snapshot ID strings.
3. `data_manifest`: 32 digest bytes as an array of JSON integers.
4. `return_definition`: validated protocol enum integer.
5. `provenance`: six byte arrays, ordered source, operators, configuration,
   data manifest, trading calendar, environment.
6. `seed`: 32 digest bytes as an array of JSON integers.

The profile uses ASCII identifiers, fixed field order, no maps, whitespace,
floating-point values or unknown fields. It is an internal lookup key, not a
replacement for canonical expression/spec/result identities. Existing reference
resolution must prove the factor, data roles and frozen fingerprints; equality
of caller-supplied hashes is not sufficient authority.

Coverage, deterministic-filter and performance rejections are reusable. Other
codes remain recorded but do not block because this key cannot prove their
dynamic evidence is unchanged. Changing a job/run ID or increasing the budget
does not bypass memory. Changing real frozen inputs creates a different context
and preserves the old evidence; this is not holdout readmission.

## Acceptance

Run the existing local TLS PostgreSQL fixture; never use the production URL:

```bash
bash scripts/postgres-test.sh start
./scripts/cargo.sh test --locked --offline -p loopd --test durable_rejection -- --test-threads=1
./scripts/cargo.sh test --locked --offline -p loopd --test durable_processes -- --test-threads=1
./scripts/cargo.sh test --locked --offline -p loopd --lib killed_writer_preserves_atomicity -- --test-threads=1
bash scripts/postgres-test.sh stop
just check
```

The process matrix covers 2/4/8 independent completions, completion retries and
blocked new submissions. Kill/restart verifies that projection, terminal job,
receipt and audit are committed together. Before commit, retry records one
rejection; after commit, retry returns the original receipt. A corrupt selected
projection/source fails closed instead of admitting work or fabricating a result.
The v1 context key is pinned by a fixture golden derived independently with
Node.js `JSON.stringify` and `node:crypto`, detecting accidental format drift.
Database-owner tampering that removes/rekeys all index entries is outside this
lookup guarantee; immutable DML guards, schema-owner separation and backup
verification remain necessary. Receipt replay detects missing projections.

## Scope And Recovery

This step does not implement durable perturbation/Sharpe history, generalized
expression failure memory, empirical trial accounting, unified readmission,
trusted reference registries, transport authentication or numerical workers.
It cannot close Phase 4 by itself. The lookup never reads holdout job results.

The production database is unchanged. Migration 7 refuses an upgrade over
existing unindexed development rejections. Once deployed, disable writers and
retain its immutable records during rollback; use a schema-aware compatibility
build or forward fix, not a destructive down-migration or blind old-binary
downgrade. Detailed decision: `docs/adr/0015-development-rejection-memory.md`.
