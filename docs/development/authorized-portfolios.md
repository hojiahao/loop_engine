# Authorized portfolio execution

`JobService.ExecuteBacktest` connects the installed numerical producer to the
existing mTLS identity, read-only data views, PostgreSQL trial history and
lease-fenced result registration. This is a development research workflow, not
an unattended search scheduler or permission to use protected samples. See
[ADR 0032](../adr/0032-authorized-portfolio-execution.md) and the
[acceptance record](../verification/phase-07-primary-backtest.md).

## Prepare the frozen inputs

1. Freeze all nine policy roles, including the portfolio/cost/execution profile
   and `daily-statistics.1` settings, before factor evaluation. Use the actual
   installed source/environment manifests. A policy added afterwards changes the
   FactorSpec and requires a new evaluation.
2. Include the factor panel and execution tape in the same immutable dataset.
   Exactly one snapshot contains the factor panel; additional execution snapshots
   contain the tape, observations and, for the v2 profile, every referenced source
   capture/raw record. All those artifacts must fit the broker's existing
   128-artifact/256-MiB view limit. Missing references do not fall back to the
   private evidence store. These bounds currently preclude large full-market
   datasets; a measured, reviewed streaming extension is a later requirement.
3. Register and execute the factor job through `EvaluateFactor`. Its persisted
   trial must be `ready_for_backtest`. Preserve the exact work Protobuf, factor
   values and evaluation manifest in the development evidence CAS. Copying files
   does not establish authority: execution verifies them against the registered
   predecessor and recomputes their numerical content.
4. Prepare the existing `loop.portfolio-request/v1` recipe described in
   [portfolio replay](portfolio-backtest.md). Set the portfolio configuration's
   `backtest_engine_version` to `authorized-portfolio.1` and its optional
   `portfolio_request` to the recipe's exact `{sha256, byte_size}`. Publish a new
   context and `loop.backtest-spec/v1` manifest. The seed, factor, dataset,
   source/environment, operators and calendar must match the factor job;
   configuration differs to bind this portfolio recipe and engine.
5. Register the Backtest job with that provenance using the existing trusted
   administrative submission boundary. Pin its exact envelope and recipe in the
   runtime configuration. There is no generic public submission RPC in this
   delivery. Pre-register the intended batch before computing portfolio results:
   later job registrations or acquired retries invalidate its statistical count.

## Enable the producer

Extend the private [runtime configuration](runtime-authority.md) with:

```json
{
  "portfolio": {
    "python": "/opt/loop_engine/.venv/bin/python",
    "output_store": "/var/lib/loop-engine/portfolio-output",
    "jobs": [
      {
        "job_id": "job.portfolio.example",
        "evaluation_job_id": "job.factor.example",
        "specification": {
          "sha256": "sha256:<actual-backtest-manifest-digest>",
          "byte_size": 1234
        },
        "request": {
          "sha256": "sha256:<actual-recipe-digest>",
          "byte_size": 2345
        }
      }
    ]
  }
}
```

This fragment contains placeholders; it is not a runnable deployment. Both
references must resolve from the configured `development_store`. The output
directory must already exist, be owned by the runtime with mode 0700, and be
disjoint from development, protected, view and factor-output stores. Missing
configuration, a changed directory identity or unresolved reference denies work.

The fixed `loop_research.portfolio_worker` module runs with isolated imports,
an empty inherited environment, one concurrent process per executor, bounded
stdin/stdout, and at most 180 seconds or the lease's remaining lifetime. The
shared runtime RPC timeout is 240 seconds. Preparation consumes the original job
deadline. Cancellation drops/kills the child; uncommitted database changes roll
back. This trusted installed worker is not an arbitrary Agent-code sandbox.

## Execute, inspect and export

The generated Rust/TypeScript/Python JobService clients expose these RPCs. TLS
terminates at `loopd`; caller-supplied actor metadata must match the certificate.

| RPC | Authorized caller and inputs | Result |
| --- | --- | --- |
| `AcquireJobLease` | Pinned research identity; job, revision, duration, context | Existing lease-fenced job |
| `ExecuteBacktest` | Same research identity; context, job ID, lease ID, revision | Completed job with immutable artifact references |
| `ReadBacktest` | Pinned operator; job ID and exact current-context SHA-256 | Reconstructed current result metadata |
| `ExportBacktest` | Pinned operator; same inputs plus context and deadline | Current metadata with durable audit/receipt |
| `DecideFactor` | Pinned operator; source job, context, factor revision, reason and deadline | Shared admission result, or an explicit unresolved-gate error |

The execute request cannot contain output metrics, an executable, a file path or
a selected trial list. Ordinary `CompleteJob` cannot import successful portfolio
outputs or a factor-rejection vote. Infrastructure failures remain operational
outcomes and do not become empirical factor failures.

The result includes values, targets, orders, fills, NAV, simple returns, costs and
exposures, plus positions, the accounting receipt, statistics and global testing
evidence. The metadata result and job completion, immutable receipt and audit
append commit atomically. CAS files left by an interrupted producer are not a
registered result. Replaying a committed request reconstructs existing evidence
without changing files or adding another completion audit.

Current reads, exports and admission reconstruct the installed calculation and
verify all referenced bytes and provenance. Export/admission retain their
30-second command bound, so an expensive cold reconstruction can return a
deadline error. An old receipt never bypasses freshness or access checks.

## Global search accounting

The trial commitment includes every FactorEvaluation and development Backtest
job in this database, across runs. Its identities bind the original job bytes,
run, factor and `max(1, acquired_attempts)`. Queued and cancelled jobs count at
least once; acquired retries add attempts. Factor and portfolio jobs are counted
separately as a conservative bound, not claimed to be independent hypotheses.
Heartbeat/revision changes and normal completion do not alter this commitment.

The reader authorizes every member and checks its immutable trial index. A
missing index, inaccessible run, corruption, more than 4096 jobs or more than
65536 attempts fails the whole operation. There is no successful-only filter,
silent truncation or count reset by starting a new run. This is database-local
completeness, not evidence about experiments performed outside this database.

For the current portfolio's NAV-mean p-value, the global report supplies the
conservative BY upper bound `min(1, p * m * H_m)`, with the other attempts assigned
p=1. These assignments do not label failed or unavailable jobs as empirical
losses. Global DSR/PBO are explicitly unavailable without a complete synchronous
return matrix. The independently computed declared-family diagnostics remain
labeled with their narrower scope; do not substitute them for global evidence.

The completion/export/admission transaction rechecks the commitment under the
existing writer lock. New jobs or acquired retries make prior results stale
without changing historical artifacts (`trial_history_changed`). Do not delete failed trials or rewrite
an old result to restore current status. Prepare a new registered run/evidence
through the same gates when recomputation is required.

## Admission and recovery

Both ordinary admission and re-admission use the existing `DecideFactor`
handler. Genuine primary evidence reaches its current-result/lineage checks;
missing independent reconciliation returns `FailedPrecondition` with the typed
`independent_validation_pending` code. It cannot be confused with an infrastructure
timeout. Phase 8 verification
and later semantic review are still required. No factor is admitted, rejected or
retired merely because this dependency is unavailable, and an override cannot
waive it. All current output remains development-only.

To roll back, stop new work and disable the optional `portfolio` configuration.
Keep all jobs, trial history, CAS artifacts, receipts and audit events. No schema
migration or destructive down-migration is needed. An older reader cannot
understand the new producer format; retain a compatible reader or forward fix.

Acceptance uses the installed worker over real mTLS and a disposable PostgreSQL
database, plus independent 2/4/8-process completion races and kill/restart around
commit. It requires no market subscription, paid API call or holdout unlock.
