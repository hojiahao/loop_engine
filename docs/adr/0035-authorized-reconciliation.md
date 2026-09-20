# ADR 0035: Authorized independent reconciliation and admission evidence

- Status: In implementation; acceptance and publication pending
- Owner: hojiahao
- Extends: ADR 0032, ADR 0033 and ADR 0034

## Requirement

Phase 8 unit 3 must turn the two administrative diagnostics into registered,
authenticated evidence. A copied success receipt, a caller-selected tolerance,
or an unregistered primary result must never open factor admission. Existing
primary results and all historical audit records remain immutable.

## Decision

Reuse the existing reconciliation job kind, PostgreSQL job/command receipts,
lease fencing, artifact store and audit transaction. Add a versioned input form
that identifies one registered primary job and its frozen context; retain the
old pair-of-backtests input for historical protocol compatibility. The new form
requests both installed validators, not a caller-selected subset. Its exact
comparison policy is deployment-resolved and checksum-bound. No new service,
database table or numerical library is needed.

The mTLS execution command accepts only attribution, job, revision and lease.
Before calculation, authorize the primary source and verify its registered
result, factor lineage and full trial commitment. Reconstruct primary evidence,
export raw observations and run the independently locked Alphalens and Zipline
processes under a bounded supervisor. Clear inherited credentials and reject
protected inputs. Preserve every numerical disagreement and unavailable result;
neither is a successful economic factor rejection.

The combined export reuses the statistics workflow's live verified primary
object for both raw-input exporters. It does not independently reconstruct the
same primary again for each exporter. Retain the original guards and check them
before final publication; standalone export commands still reconstruct their own
inputs. No calculation or evidence is reused across different operations.

Bind the final receipt to the primary job/revision/result, frozen comparison
policy, both actual validator receipts and their build identities. Register it
through the existing completion transaction only after rechecking source state,
file guards, current authority and the live lease. A generic completion cannot
import an administrative receipt as authorized evidence. Interrupted execution
may leave unreferenced immutable files, but no completed job or success audit.

Current reconciliation reads and command retries reconstruct all evidence and
deny source, trial, policy, build or artifact drift. Admission and readmission
use the same existing handler and require the bound registered reconciliation.
An accepted numerical comparison cannot waive the separate licensed-data,
complete statistical-search and completed semantic-review prerequisites.
The currently supported synthetic/public-development profiles remain ineligible
for production admission; their independent reports are still useful and readable.

Independent-evidence decisions have an explicit maximum request span of 180
seconds, covering primary and two independent replays before the database
decision. The inherited 30-second metadata-decision budget expired during the
actual installed-worker acceptance path after successful reconciliation and
restart replay. Ordinary decisions keep 30 seconds; only the runtime's
non-serializable supervised proof enables the longer storage envelope. Requests
above 180 seconds, elapsed deadlines and clock regression still deny. Database
statement/lock limits and job lease limits are unchanged.

The new integration matrix repeatedly executes both installed independent
engines, including 2/4/8 distinct writer processes and crash recovery. Keep all
cases in the existing CI jobs and provide bounded total-job budgets of 60 minutes
for Rust, 75 for the unified workspace and 90 for the clean development container.
These are test/build capacity limits, separate from the unchanged per-operation
runtime bounds. Record the actual exact-commit CI durations before acceptance.

## Acceptance

Exercise actual installed validators through mTLS and PostgreSQL, including an
accepted numerical comparison and immutable restart replay. Reject role/actor
confusion, missing deployment, protected inputs, unregistered sources, imported
success, stale trials/builds/artifacts, altered policy, expired leases and
cancelled jobs. Preserve mismatch/unavailable diagnostics without admitting a
factor. Check admission/readmission and semantic override cannot waive missing
gates. Reuse the existing transaction/process infrastructure for competing
writers and kill/restart at commit boundaries. Extend protocol compatibility and
three-language validators for the added input form and execution/read messages.

## Recovery

Disable the optional reconciliation deployment to stop new writers; keep old
primary results, independent receipts, jobs and audit events. Old readers must
deny unsupported new input forms rather than reinterpret them as legacy pair
comparison. Roll back executable changes without deleting research records or
performing destructive down-migrations. Record task tests, commit, push and
exact-commit CI before accepting this decision.
