# Factor admission and trial accounting

`FactorRepository::decide_factor` is the internal Rust command for both ordinary
admission and readmission. It consumes registered current primary IS results;
it neither submits a new backtest nor accepts user-supplied performance values.
Production `BacktestPolicy` still denies report resolution and semantic override.
There is no enabled external admission RPC or `loopctl factor admit` command yet.

## Decision contract

The request names a source job, immutable context, expected factor revision,
reason, idempotency key and a deadline within 30 seconds. Revision zero means
first consideration; later consideration must name the current revision. An
already admitted factor cannot be admitted again. A rejected/retired factor
needs a new result job, except for an explicitly authorized semantic override.

Both paths execute the same checks: separately authenticated actor, development
job/trial binding, primary result, all six current fingerprints, server-resolved
IS report/policy, valid/eligible coverage, machine gates, completed semantic
review and the reviewed active library. Coverage compares integers using u128:
`valid * 10000 >= eligible * minimum_coverage_bps`. Zero eligible observations,
counts outside the eligible set, missing reports and unresolved policies are
invalid/unavailable evidence, not an investment conclusion.

The active-library digest is SHA-256 over ASCII `loop.active-library.v1`, a NUL
byte, and compact UTF-8 JSON of sorted `[factor_spec_id, revision]` pairs. It
contains admitted factors only, with a hard limit of 4096. Reports bind the
context, this snapshot, frozen rules and exact primary result. Replacements are
at most sixteen sorted distinct IDs from that report; the command body cannot
nominate arbitrary factors to delete. Numerical correlation and superiority
criteria belong to the report's pinned research policy, not Rust storage code.

An override requires a human principal, explicit permission, nonempty reason
and independently resolved approval bound to that principal/report/reason.
Only a completed semantic rejection may be waived. No machine/data/provenance
gate is bypassed. The immutable receipt preserves requested/applied status and
the original report; `override_authorized` and decision events commit together.
The command audit explicitly identifies standard/readmission path and applied
override. Retirements use `loop.factors.retire` command audit records and retain
the displaced factor's original evidence job.

Each factor/context has monotonic revision and lifetime counters. `admitted`
means `admissions = retirements + 1`; `retired` or `rejected` means the counts
are equal. Readmission increments admissions without resetting retirements.
All displaced state updates, new state, immutable receipt and audit append are
one transaction. Any failed replacement, audit error or cancelled command rolls
back the entire decision. Identical retries recheck authority and current
source evidence, returning the original receipt without a second retirement.
Such a historical response does not mean the factor is still currently active.

## Trials

`insert_job` registers factor-evaluation and development-backtest trials before
the submission transaction commits. Queued work has attempt zero; successful,
rejected and infrastructure-failed jobs retain their actual execution attempts.
Pre-execution cancellation stays distinguishable from an attempted trial. No
protected job is included. A rejected submission is not falsely recorded as a
completed numerical trial; later Loop Runtime events will record pre-job search
proposals and validation/duplicate skips separately.

`factor_trials(principal, run_id, after_job_id, limit)` pages up to 500 verified
records under per-job read authorization. It includes negative/operational
outcomes instead of counting only admitted factors. The trial input checksum
detects corruption; the canonical FactorSpec ID remains the research identity.
Job executions and admission decisions are distinct accounting units, so a
semantic override does not create another numerical experiment.

## Verification and recovery

```bash
bash scripts/postgres-test.sh start
./scripts/cargo.sh test --workspace --all-features --locked --offline \
  --test durable_library --test durable_processes --lib -- --test-threads=1
bash scripts/postgres-test.sh usage
bash scripts/postgres-test.sh stop
```

These tests use real PostgreSQL jobs/receipts and explicitly fabricated IS
report metadata, not licensed market data. Phase 4 units 3-5 must supply trusted
manifest parsing, deployed identities, data isolation and actual evaluation.
Independent backtester admission requirements belong to Phase 8/13; this
internal boundary does not assert production factor readiness.

Keep the workspace feature selection when running targeted tests: `-p loopd`
alone enables fewer protocol features and builds a separate, large test cache.

Migration 9 adds two projections and refuses to fabricate trials for existing
research jobs. It is not deployed to production. Disable decision/submission
writers before rollback; keep all trials, factor states, receipts and audit.
After schema deployment use a schema-aware recovery build or forward fix,
not table deletion or removal of a migration checksum. See ADR 0017.
