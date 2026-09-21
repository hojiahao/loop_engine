# ADR 0037: Bind global statistics to independent reconciliation

- Status: Implemented; local acceptance passed, publication and remote CI pending
- Owner: hojiahao
- Extends: ADR 0035 and ADR 0036

## Requirement

A candidate's independent comparison must identify the registered, complete
search-population report used by admission. A successful comparison, an imported
JSON report or an old primary's smaller trial count cannot substitute for that
evidence. Missing, stale and unavailable statistics must remain distinct.

## Decision

Extend the frozen comparison policy with a version 2 form that names one global
Report job. Keep version 1 bytes and historical reports unchanged. Reuse the
existing authenticated statistics replay, independent workers and completion
transaction; introduce no service, table, RPC or numerical threshold.

The runtime authorizes and reconstructs the registered report over the full
current trial population. The candidate must be one of its exact registered
portfolio sources. Bind report job/revision/manifest/summary and the candidate's
strategy identity to the new reconciliation receipt. Statistical availability
means the complete matrix and candidate BY/DSR plus population PBO are available;
it does not mean the strategy passes an economic acceptance threshold.

Before completion, read/replay or admission, recheck the report registration,
entire trial snapshot, every source, numerical provenance and live file guards
inside the existing transaction. New trials invalidate the comparison without
rewriting history. An original primary may retain its older trial commitment
only along this bound global-report path; ordinary primary reads remain strict.

Reuse one verified primary reconstruction within each independent export
operation. Both independent engines still calculate their own results. Do not
cache authority across operations or increase runtime/lease deadlines.

The shared admission/readmission handler checks the bound report before any
semantic override. Missing or unavailable statistics deny with separate reasons.
Available diagnostics still cannot waive licensed data, a frozen economic
acceptance decision or completed semantic review. All supported profiles remain
development-only and no factor is newly admitted by this change.

## Acceptance and recovery

Exercise real multi-strategy statistics followed by Alphalens/Zipline over mTLS
and PostgreSQL, including an older primary, immutable restart replay, ordinary
and forced admission denial. Cover missing reports, wrong candidate membership,
unavailable metrics, stale populations, corruption and transaction-time drift.
Keep existing process/crash tests and v1 replay compatibility.

Disable the optional reconciliation deployment to stop new writes. Preserve all
jobs, receipts, reports and audit events. No down-migration is required. An older
executable rejects the unsupported v2 policy; restore a compatible reader or
forward fix to inspect v2 evidence. Publish local and exact-commit CI evidence
before accepting this delivery unit.
