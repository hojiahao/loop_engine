# ADR 0036: Authorized global strategy statistics

- Status: Accepted; e325368 is pushed, CI 35569521771 passes all seven jobs
- Owner: hojiahao
- Date: 2026-09-20
- Extends: ADR 0031 and ADR 0032

## Requirement

The current authorized primary report records every research job/attempt and a
conservative BY bound, but always reports global DSR/PBO as unavailable. The
administrative complete-family calculator cannot authenticate the full database
population. Complete this missing path without rewriting historical primary
reports, choosing a profitable subset or treating infrastructure failures as
observed strategy losses.

## Decision

Reuse the existing Report job kind, PostgreSQL lifecycle/leases/receipts/audit,
the installed Python worker, immutable artifacts and the ADR 0031 kernels. Add
narrow authenticated execute/read operations and an optional deployment-pinned
report recipe. No new service, database table or numerical dependency is needed.
RPCs carry attribution, job identity, revision and lease only. The server reads
the whole development trial registry, authorizes every member and commits to
each persisted revision and outcome. Report jobs never enlarge the search count.

Resolve each successful portfolio to its registered factor-evaluation predecessor
and its original immutable receipt. Only installed-producer, development-only
evidence with current numerical provenance is eligible. Replay each source with
its original trial commitment; require every historical trial identity to exist
unchanged in the current registry and attempt counts never to decrease. Refresh
global statistics using the entire current population, without presenting the
old source's search-adjusted statistics as current. Ordinary primary-result
reads retain their stricter original-ledger freshness gate. Every factor trial
must have a verified portfolio continuation; pending/rejected/failed jobs, missing historical
retry results, unmatched factors and incompatible samples make the global
matrix unavailable. They stay in the report and conservative denominator.
Never manufacture returns for them or trim observations to align histories.

A matrix column represents a frozen strategy configuration, not an orchestration
job. Registered evaluation and portfolio steps share one strategy lineage.
Deterministic duplicate configurations are identified explicitly and must have
identical returns; they are not independent columns. Retain all job and attempt
counts separately. DSR retains the documented all-distinct-strategies-independent
sensitivity assumption; it is not an estimate of effective independent searches.
CSCV uses every equal-block split over the exact common date axis. Missing or
degenerate statistics remain unavailable, never automatic admission.

Calculation and replay revalidate source bytes/builds and the frozen statistical
policy. Within one operation Rust verifies registered source bytes and lineage;
the Python worker numerically reconstructs every successful portfolio exactly
once, including when another unfinished trial makes the matrix unavailable.
The result is not authority until that reconstruction succeeds. Input reads,
calculation and output reads have separate bounded budgets under the enclosing
operation deadline; a file-read budget must not include numerical runtime.
The existing completion transaction rechecks the full trial snapshot,
all registered sources, authority, file guards and lease before appending the
report, receipt and audit event together. Generic completion cannot import a
report as proof. Current reads repeat freshness checks without repairing files.
All scans, matrices, subprocesses and database waits are bounded. Historical
records and reports remain immutable when new trials make them stale.

The report is statistical evidence only. Phase 8 binds it to independently
reconciled candidates; licensed data and later completed semantic review still
gate production admission. No live trading, paid data access or holdout unlock
is introduced.

## Acceptance and recovery

Exercise actual registered multi-strategy portfolios over mTLS/PostgreSQL, exact
dates, complete membership, deterministic deduplication, numerical goldens,
unfinished/failed/retried work, incompatible contexts, source drift, unprivileged
roles, imported success, cancellation, immutable replay and transaction fencing.
Preserve existing process/crash regression coverage. Record local and full remote
gates before closing the delivery unit.

Disable the optional report deployment to stop new writers. Retain old primary
results, reports, trial records and audits. No destructive down-migration is
needed. Older executables must refuse the unsupported report recipe.

## References

- [Deflated Sharpe Ratio, including independent-trial assumptions](https://www.davidhbailey.com/dhbpapers/deflated-sharpe.pdf)
- [Probability of Backtest Overfitting, complete performance matrix and CSCV](https://www.davidhbailey.com/dhbpapers/backtest-prob.pdf)
