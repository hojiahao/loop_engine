# Evaluation trials and admission lineage

Phase 6 unit 3 connects the authorized numerical worker to durable research
bookkeeping. It does not implement portfolio construction. See ADR 0028 and the
Phase 6 verification record for the acceptance status.

## Executable path

Use the existing administrative panel workflow, freeze its FactorSpec and
research context, then deploy the existing mTLS job runtime as described in
`factor-evaluation.md`. No new public service or general-purpose submit endpoint
is added. Job creation and trial reads remain internal Rust repository commands;
an Agent cannot nominate an executable, path, capability or arbitrary result.

1. Submission atomically registers an accepted factor job and `factor_trials`
   row. Replaying the same submission returns its original receipt.
2. Submission and queued-job lease acquisition check prior completed evaluations
   under the existing PostgreSQL ledger lock.
3. `JobService.EvaluateFactor` resolves the actual frozen policy, computes using
   the installed Python worker and verifies its immutable values and manifest.
4. Completion atomically records the job, immutable `factor_evaluations`
   projection, completion receipt and state-transition audit.
5. `FactorRepository::factor_trials` returns each accepted job's current state,
   attempt count and optional verified `EvaluationTrial`. Its result references
   bind the actual output manifest, values, policies, seed and six fingerprints.
6. A later registered primary IS backtest and version-2 review can call the
   existing `decide_factor` command. Ordinary admission and readmission share
   this handler, revisions, retirement counters and semantic-only override rules.

## Coverage and failure memory

The FactorSpec's evaluation policy must contain `minimum_coverage_bps` as a
canonical integer string in `1..10000`. Missing, zero, negative, fractional,
leading-zero or out-of-range thresholds fail before numerical execution.
No caller or LLM can supply a replacement threshold in the evaluation RPC.

Coverage passes exactly when the eligible count is positive and
`valid * 10000 >= eligible * minimum_coverage_bps`. The comparison uses `u128`,
so large counts do not overflow or introduce floating-point boundary errors.
Warmup stays excluded; transformed coverage uses the final finite values.

| Numerical job | Evaluation disposition | Subsequent identical request |
| --- | --- | --- |
| Succeeded with sufficient coverage | `ready_for_backtest` | `AlreadyEvaluated`; no new trial |
| Succeeded with insufficient coverage | `insufficient_coverage` | `PreviouslyRejected`; no new trial |
| Failed, cancelled or exhausted | No numerical evidence | No empirical failure is fabricated |
| Successful before migration 0010 | Unverified historical baseline | Requires fresh authorized evaluation |

`Succeeded` means the computation completed correctly, not that its factor
passed research gates. Neither disposition grants factor admission. Raw
`CompleteJob` calls cannot fabricate numerical success or a coverage rejection.
Errors caused by files, source drift, authentication, transport or the worker
remain operational failures. Historical completed-backtest rejection memory
continues to handle its existing deterministic coverage/filter/performance codes.

The evaluation lookup key includes the canonical FactorSpec ID, ordered
snapshots, data-manifest digest, all six provenance components and seed. Changing
a job ID, run ID, idempotency key or budget cannot bypass it. A different frozen
context is a new study input. Already executing work may finish and remains
counted; there is no claim of global in-flight uniqueness. A blocked queued
job remains visible with its existing attempt count until its scheduler cancels
it; a skip must not be recorded as another factor failure.

RPC errors expose no prior job identity or artifact reference. `PreviouslyRejected`
is `FailedPrecondition` and `AlreadyEvaluated` is `AlreadyExists`; both carry a
nonretryable conflict detail. The scheduler must skip/escalate them rather than
treating them as a retryable provider or storage outage.

## Admission evidence

New file-backed reports use `loop.admission-review/v2`, artifact schema version
2, with an additional `evaluation` object containing `job_id` and the immutable
result `manifest` reference (`sha256` and `byte_size`). All existing review
fields remain mandatory. The actual manifest is materialized before opening
the decision transaction, and its file guards remain checked during resolution.
Version-2 preparation uses the numerical resolver's existing 30-second bound
for cold verification of the full native environment, instead of the legacy
10-second metadata-fixture budget. Command deadlines remain enforced; timeout
creates no decision, trial result or factor rejection.

The shared handler authorizes access to the referenced numerical job and
verifies its immutable trial/result projection. It requires matching factor,
dataset, seed, actual value content/schema, coverage counts, threshold and
evaluation policy. Source, environment, operator, data and calendar fingerprints
must match the backtest; configuration can differ because it names a different
engine. The common FactorSpec still freezes all nine policy identities.
Missing evidence and mismatched lineage are errors, not negative LLM votes.
Force cannot bypass these checks or the exact coverage gate.

Version-1 review artifacts remain readable and exportable historical metadata;
the deployed `TrustedManifests` resolver refuses new admission from them.
Explicitly fabricated `BacktestPolicy` test doubles retain their old isolated
decision-accounting tests and do not prove numerical or market performance.
Production integration uses the actual-file resolver; a custom trusted policy
implementation is part of the authority boundary, not caller input.

The integration tests use genuine computed factor values and explicitly
synthetic portfolio/review artifacts. Their admitted fixture state proves the
shared handler's wiring only. A formal US portfolio producer, transaction costs,
portfolio statistics and independent reconciliation remain later phases.

## Verification and rollback

After bootstrap, run the focused behavior against the disposable local TLS
PostgreSQL fixture:

```bash
bash ./scripts/postgres-test.sh start
CARGO_BUILD_JOBS=1 ./scripts/cargo.sh test --locked --offline --workspace --all-features --lib evaluation -- --test-threads=1
bash ./scripts/postgres-test.sh stop
```

The `evaluation` filter includes source-to-worker regressions, actual numerical
workflow, exact coverage/context cases and independent 2/4/8-process and
kill/restart acceptance. Then run the repository's `just check`, `just test`,
`just build`, `just doctor` and isolation gates. Test fixture URLs reject a
production database name/principal. Remove only identified project test output
after its evidence is recorded.

Apply migration 0010 only through the existing deployment migration role with
new writers stopped. It adds one immutable projection and records preexisting
successful numerical jobs as baseline rows without reinterpreting their opaque
protobuf results. Existing jobs, trials, receipts and audits keep their original
bytes. Such baseline rows cannot supply new admission or failure/duplicate
evidence. New successful numerical writes require an atomic projection at the
database boundary. Existing application grants for newly created tables apply;
the runtime remains unable to apply DDL.

Rollback disables evaluation/new-admission writers and retains the additive
schema and research history. Use a reader compatible with migration 0010; older
binaries can refuse the newer migration. Do not drop the projection, rewrite
receipts, relabel historical coverage or use a destructive down-migration.
