# ADR 0017: Unified factor admission and trial accounting

- Status: Accepted internal command; production denied
- Date: 2026-09-11
- Owner: hojiahao

## Decision

Phase 4 delivery unit 2 replaces the old parallel admission/readmission paths
with one Rust command over already registered development backtest evidence.
It does not run a model or calculate a backtest inside a database transaction.
Trusted artifact resolution is delivery unit 3, runtime identity/isolation unit
4 and numerical execution unit 5. Defaults deny unresolved evidence throughout.

Every accepted factor-evaluation or development-backtest submission atomically
registers a trial alongside its job. The index retains canonical FactorSpec ID
and the exact original specification checksum. Queued, executed, rejected,
cancelled, exhausted and infrastructure-failed work remain distinguishable by
the source job's state and attempt. Replay creates no extra trial. Denied RPCs
and skipped duplicates are not fabricated execution trials. The index never
includes protected jobs or silently imports A-share checkpoints.

`decide_factor` checks transport attribution, current registered primary IS
evidence, immutable policy/report binding, exact valid/eligible coverage counts,
deterministic filters, semantic review and the active-library snapshot. Ordinary
admission and readmission differ only in prior persisted state/revision; neither
can skip a gate. Readmission retains the earlier receipt, rejection and counters.
Infrastructure or unavailable review evidence fails without a research decision.

A human override requires a nonempty reason, independently resolved approval
reference and explicit permission. It may waive only an actual semantic-review
rejection. Coverage, deterministic rejection, stale inputs, holdout isolation,
missing evidence, active duplicate and CAS conflicts remain non-overridable.
Receipts record requested/applied override status; the same transaction appends
the dedicated override audit event and the actual admission decision.

Replacement candidates come from trusted review evidence, never arbitrary IDs
in an Agent command. A bounded, sorted snapshot of active IDs/revisions is bound
to that evidence. Under the ledger lock, replacement retirement and admission
commit together. Per-factor lifetime admissions and retirements only increase;
retired entries are retained, not deleted. Immutable receipts preserve all
rejected/readmitted/replaced states and their source reports.

Two tables suffice: an immutable trial index and revision-fenced factor state.
Existing receipts and audit supply history; no new service, queue, counter
framework or numerical dependency is introduced. Commands, lock waits, library
scans and trial pages have explicit bounds. Semantic request replay reauthorizes
and rechecks provenance but returns the original receipt without reapplying
retirements or issuing another decision.

## Acceptance And Recovery

Implementation `365875d` is pushed. Local `just check/test` and all seven jobs
in GitHub Actions run `34566128777` passed. This closes delivery unit 2, not
Phase 4, production deployment or real-data research validation.

Test real job submission/lease/completion followed by ordinary admission,
rejection, readmission, human override, atomic replacement and restart. Include
coverage boundaries, failed/absent review, every stale fingerprint, denied and
spoofed actors, duplicate source decisions, CAS/key conflicts, corruption,
immutable SQL guards, rollback, clock/deadline/cancellation, 2/4/8 OS processes
and kills before/after commit. Fixture evidence remains explicitly synthetic;
passing it does not enable a live research engine.

Migration 9 must refuse existing research jobs lacking a verified trial import;
SQL cannot safely derive canonical IDs from opaque Protobuf blobs. The last
production verification recorded no such jobs; deployment must recheck this
precondition. This task does not modify production. Existing migrations are
immutable. Before deployment revert the delivery normally. After deployment
disable writers and retain migration, trials, states, receipts and audit; use a
schema-aware compatibility build or forward fix, never a destructive downgrade.
