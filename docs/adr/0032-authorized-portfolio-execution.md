# ADR 0032: Authorized portfolio execution and global trial accounting

- Status: Implementation in progress; publication and remote gates pending
- Owner: hojiahao

## Requirement

Phase 7 unit 4 connects the installed portfolio producer to the existing mTLS,
data-view, lease, result, export and shared admission boundaries. Administrative
receipts and selected successful experiments must not become authorized research
evidence merely by being copied into an object store.

## Decision

Reuse JobService, PostgreSQL jobs/factor_trials/backtest_results, the immutable
object store and the installed Python numerical package. Add one explicit
development-backtest RPC and a deployment-pinned producer. The caller supplies
only command attribution, job, revision and lease; never metrics, paths, code or
an experiment subset. The producer reconstructs registered factor evidence,
computes the portfolio and statistics, and publishes immutable artifacts. Only
the existing lease-fenced completion transaction can register the result.

Execution observations are part of the frozen dataset and read through the
broker's read-only view. Protected samples remain denied. A current read/export
reconstructs numerical evidence and checks all original fingerprints, artifact
bytes and the global trial commitment before releasing metadata.

The trial scope is every development research job in this database, across runs,
not a caller-selected family. Verify the immutable trial index and job projection;
count each accepted job at least once and every acquired retry attempt. Count
queued, cancelled, rejected and infrastructure-failed jobs conservatively. Do not
call those states empirical losses. Bound the scan and fail on overflow or any
unauthorized member; never silently truncate. A changed membership/attempt count
invalidates current statistics. Check the commitment again under the existing
global writer lock during result registration, export and admission.

Use the conservative BY adjusted-p upper bound `min(1, p * m * H_m)` for the
current trial, where all other globally registered attempts receive p=1. This
does not pretend to reconstruct a synchronous return matrix or estimate the
effective independent search count. DSR/PBO remain explicitly unavailable for
the global history until that matrix exists; declared-family diagnostics from
unit 3 remain separately labeled. Never admit on a missing statistic.

The shared admission command consumes genuine registered numerical lineage.
Phase 8 independent reconciliation remains a mandatory unresolved gate; neither
successful primary execution nor a semantic override can waive it. This unit
does not claim production data eligibility or independent validation.
The pending gate has a typed non-retryable precondition error, distinct from a
dependency timeout. Trial-history drift also has its own typed conflict code.

## Verification and recovery

Exercise installed numerical execution over mTLS and PostgreSQL, imported-output
denial, actor/lease/role confusion, protected-data denial, global trial changes,
read/export freshness, shared admission, corruption, cancellation and restart.
Keep the independent-process 2/4/8 writer and commit-boundary crash matrices.
Record precise local and remote evidence before marking the unit complete.
Concurrency fixtures use one post-computation logical commit clock; expiry tests
retain the live runtime clock. No production deadline is relaxed for slow tests.

No new service, numerical dependency or database migration is required. Disable
the optional portfolio deployment to stop new writers. Preserve all jobs,
artifacts, receipts, trial history and audit events. Older builds cannot consume
the new producer's result format; retain a compatible reader or forward fix.
