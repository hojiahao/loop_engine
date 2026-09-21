# Authorized independent reconciliation

The optional runtime connects a registered primary portfolio to both installed
independent validators. It records a durable comparison, not permission to trade
or a production-ready factor. See [ADR 0035](../adr/0035-authorized-reconciliation.md),
[global report binding](../adr/0037-statistical-reconciliation-binding.md)
and the [Phase 8 acceptance record](../verification/phase-08-independent-validation.md).

## Inputs and deployment

Complete [authorized portfolio execution](authorized-portfolios.md) first. Freeze
the primary job, its context SHA-256 and this exact comparison policy in the
development evidence CAS. Serialize in the displayed schema field order, without
whitespace, then calculate its actual SHA-256 and byte size:

```json
{
  "schema": "loop.reconciliation-policy/v2",
  "policy_id": "policy.reconciliation",
  "revision": "1",
  "profile": "alphalens-zipline-development.1",
  "statistics_absolute": "0.000000000001",
  "statistics_relative": "0.0000000001",
  "price_absolute": "0.000000005",
  "dollar_absolute": "0.00001",
  "return_absolute": "0.000000000001",
  "accounting_relative": "0",
  "statistics_job": "job.statistics.example"
}
```

These constants correspond to the implemented independent comparison profiles.
Changing an arbitrary tolerance cannot relax them. A new numerical profile
requires an implementation review and new immutable policy evidence.

Before this comparison, register the named report using the
[global statistics workflow](global-statistics.md). Its complete database trial
population must include this exact primary job and all other attempted research.
The report ID is frozen in the policy, not chosen by an RPC caller or selected
after seeing which report passes. The runtime needs both optional deployments.

Version 1 policies omit `statistics_job` and keep their original serialization.
Existing v1 reports remain diagnostic history; accepted numerical agreement now
returns `global_statistics_pending` at admission. Create a new policy/job for v2;
never edit a published policy or receipt in place. V2 uses the same numerical
comparison profile and tolerances; the new version adds provenance binding only.
Serialization compatibility does not waive build freshness: source/environment
changes still make prior numerical evidence stale and require new research runs.

Submit an `INDEPENDENT_RECONCILIATION` job through the existing trusted
administrative submission boundary. Its `ReconciliationJobInput` contains:

- `validation.primary_job_id`: the registered development Backtest job ID;
- `validation.context_manifest_sha256`: the exact 32-byte context digest;
- `reconciliation_policy`: policy ID, revision and policy document digest;
- the normal bounded job budget.

Leave both legacy `primary_backtest_id` and `independent_backtest_id` absent.
Legacy pair-comparison inputs remain readable historical protocol shapes; this
runtime executes only the new single-primary form. Reconciliation jobs do not
change the search-trial denominator: they cannot propose another factor, parameter
or direction. New factor/portfolio jobs and retries still make old statistics stale.

After bootstrap has installed both independent locks, add this optional fragment
to the private runtime configuration. Replace every placeholder with actual
paths, job IDs and checked references; this example is not a runnable deployment:

```json
{
  "reconciliation": {
    "uv": "/opt/uv/bin/uv",
    "zipline_python": "/opt/loop_engine/.tools/python/cpython-3.12.13-linux-x86_64-gnu/bin/python3.12",
    "alphalens_project": "/opt/loop_engine/python/alphalens_validation",
    "zipline_project": "/opt/loop_engine/python/zipline_validation",
    "cache": "/opt/loop_engine/.tools/uv-cache",
    "output_store": "/var/lib/loop-engine/independent-output",
    "jobs": [
      {
        "job_id": "job.validation.example",
        "primary_job_id": "job.portfolio.example",
        "policy": {"sha256": "sha256:<actual-policy-digest>", "byte_size": 1234}
      }
    ]
  }
}
```

The primary interpreter remains Python 3.14.4. The secondary interpreter belongs
only to Zipline's separate lock. No second persistent `.venv` is created.
Output must be an existing, canonical, runtime-owned 0700 directory. Project,
cache and output namespaces must not overlap development, protected, broker-view,
factor-output or portfolio-output storage. One primary has exactly one pinned
reconciliation job per deployment, preventing callers from selecting whichever
report passes. Installations and configuration are trusted administrator inputs;
workers are fixed service modules, not arbitrary user code or an OS sandbox.

The runtime pins verified source and artifact file descriptors while an operation
uses them. On Linux, give its service a soft/hard `LimitNOFILE=65535`, matching
the supported local acceptance environment. The default systemd soft limit of
1,024 is insufficient for the installed numerical dependency manifests; the
runtime then denies execution with an evidence-availability error. This resource
setting does not change artifact counts, byte limits or authority checks.

## Authenticated operation

Use the generated JobService clients with the existing mutual TLS configuration.
No executable, path, output metric, selected validator or tolerance enters these
requests. The existing identity/job pinning applies to both source and comparison.

| RPC | Caller and request | Effect |
| --- | --- | --- |
| `AcquireJobLease` | Pinned research identity, comparison job and revision | Acquire the existing bounded lease |
| `ExecuteReconciliation` | Same identity, command context, job ID, lease ID, expected revision | Reconstruct primary evidence, run both validators, register final report atomically |
| `ReadReconciliation` | Pinned research identity or operator, comparison job ID | Reconstruct and validate current evidence; return job and small immutable report reference |
| `DecideFactor` | Pinned operator, original primary job, context and normal decision fields | Recheck the pinned registered comparison inside the shared admission/readmission path |

Actor fields are attribution checked against the certificate, not authentication.
Discovery, Provider and HoldoutWorker identities cannot execute this workflow.
Protected source jobs are rejected before materializing an input view. Ordinary
`CompleteJob` cannot import an independent success receipt as proof of execution.

The supervisor clears inherited credentials, uses `uv --isolated --locked
--offline`, bounds stdin/stdout to 1 MiB, and permits one validation operation per
executor. Execution and current reads have an outer 180-second bound; execution
also consumes the live lease's remaining time. RPCs retain their 240-second
bound. For `DecideFactor` with a pinned registered reconciliation, set its absolute
deadline at most 180 seconds after `context.requested_at`: this command must
replay the primary and both independent engines. Ordinary decisions retain a
30-second span. Only runtime-created supervised evidence enables the larger
storage envelope; database lock/statement timeouts and leases are unchanged.
Cold or large calculations can exceed these bounds and return an explicit operational failure. Cancellation
terminates the worker process group, including uv's Python descendant.

## Report interpretation and freshness

The final `loop.authorized-reconciliation/v2` document binds job/lease, primary
job/revision/result, context, frozen policy, prepared inputs, both receipts and
attempt start time from its persisted lease. Concurrent retries therefore bind
the same timestamp. The job and audit record hold the actual commit time.
Each independent receipt includes its actual source/dependency
build identity and detailed differences. Raw observations and their export
normalization remain shared dependencies; this is not independent vendor data
certification. See [statistics](independent-statistics.md) and
[accounting](independent-accounting.md) for numerical limits.

Its `global_statistics` member identifies the registered report job/revision,
manifest and summary, the candidate's distinct strategy binding when available,
and explicit availability reasons. The full summary retains numerical values and
the original reasons for missing metrics. No return matrix is copied into an RPC
or this reconciliation document. `available=true` requires a complete matrix,
candidate BY/DSR and population DSR benchmark/PBO availability; it is not a test
of profitability or a calibrated economic acceptance threshold.

The runtime reconstructs the entire registered report before independent export,
then checks that same report, every source and exact trial revision again in the
completion/admission transaction. A primary made before later strategies can be
used under this new global report without rewriting its old trial count. An
ordinary current-primary read continues to reject its old search-adjusted
statistics. Changed population returns `trial_history_changed`; missing completed
reports return `global_statistics_pending`; an independently agreeing candidate
with unavailable global diagnostics returns `global_statistics_unavailable`.
Corruption, authority failures and timeouts remain operational errors.

An operationally completed job may have any numerical disposition:

- `accepted`: both independent comparisons agree under the frozen profiles;
- `rejected`: at least one comparison disagrees; retain the detailed differences;
- `unavailable`: no disagreement, but a required statistic or comparison is absent.

Neither disagreement nor unavailable evidence is an economic rejection of a
factor. Missing packages, source corruption, process failure, expired authority,
deadlines and lease loss never become successful comparison receipts. Historical
primary reports retain their original `pending-phase8` label: the separately
registered reconciliation is the new evidence; old reports are not rewritten.

Current reads and retries replay both actual validators and verify every original
object without writes or repair. Before registration, the transaction rechecks
source registration, factor lineage, complete trial commitment, file guards,
authority and lease. Job state, command receipt and audit append commit together.
Corrupted files, changed builds/policies/trials and mismatched result references
make the old report unusable as current evidence while preserving history.

Within each export operation, statistics and the two raw-input exporters share
one live verified primary reconstruction and its final integrity guards. This
avoids repeated primary calculations without caching authority or skipping the
two independent numerical calculations.

All currently supported synthetic/public-development profiles have
`production_eligible=false`. Even agreement returns a separate production
prerequisite error at admission: licensed historical coverage, frozen economic
statistical acceptance and completed semantic review are still required. This
delivery binds diagnostic evidence; it does not invent acceptance thresholds or
enable production enrollment.
Ordinary admission, readmission and semantic overrides cannot waive these gates.

## Acceptance and rollback

Run the focused tests against the disposable local database, never production:

```bash
ulimit -n 65535
bash scripts/postgres-test.sh start
bash scripts/cargo.sh test -p loopd --lib reconciliation --locked --offline -- --test-threads=1
bash scripts/cargo.sh test -p loopd --lib runtime::process --locked --offline -- --test-threads=1
bash scripts/uv-research.sh run --locked --offline --no-sync pytest tests/test_reconciliation_worker.py
bash scripts/postgres-test.sh stop
```

Protocol tests share one Rust/TypeScript/Python input matrix; workspace gates
also check generated binding drift, formatting, naming and types. The integration
suite uses actual installed validators, mTLS, PostgreSQL and separate OS processes
for 2/4/8 writers and before/after-commit interruption. No paid API or holdout
unlock is involved.

Disable the optional `reconciliation` configuration and stop new writers to roll
back. Preserve registered jobs, receipts, audits and immutable CAS objects. There
is no new table or database migration. An interrupted publisher can leave an
unreferenced immutable object; it is not a completed report and must not be
promoted by hand. Only remove a demonstrably unreferenced project temporary object
after the job has stopped. Older readers must deny the unsupported input form;
retain a compatible reader or use a forward fix to inspect new history.
