# Phase 8 unit 4: Global statistical evidence binding

Status: implementation `8f30526` is published; local acceptance passed. The
standalone Rust CI budget correction still requires exact-commit acceptance
before phase closure.

## Requirement and implementation

ADR 0037 binds the authenticated full-population report to the exact candidate,
comparison policy, independent receipts and shared admission/readmission path.
The v2 policy and report preserve all v1 bytes. New trials make bound reports
stale; an old primary may participate only through the freshly replayed global
population. Registered reports remain immutable.

The change reuses existing storage transactions, authorities, jobs, artifact
guards and installed workers. No schema migration, new service, dependency or
paid request is introduced. Python exports reuse the operation's verified
primary while Alphalens and Zipline retain independent numerical calculations.
Availability is distinct from economic acceptance; production remains disabled.

## Acceptance record

The six new Rust/mTLS/PostgreSQL integration tests pass (1557.31 seconds on this
small local host, after a 4m04s test build). They run actual registered primary
strategies, the full-population reporter, Alphalens and Zipline. Cases cover:

- an older primary bound to a later complete two-strategy report, followed by
  restart/current read with unchanged report identity and audit count;
- ordinary and forced admission denied despite available global statistics;
- an independently agreeing candidate with unavailable global statistics;
- a report job that exists but has not completed;
- a completed report made stale by another registered trial;
- trial insertion after actual calculation but before completion: the job,
  command receipt and success audit remain uncommitted.

The 26 affected Python workflow tests pass in 455.90 seconds, including actual
v1 export/replay, corrupt/missing evidence, declared-family statistical cases,
and a guard against duplicate primary reconstruction inside independent export.
The original v1 Rust acceptance/restart/current-read/ordinary-readmission/forced
decision case also passes (735.58 seconds), preserving immutable report identity
and audit behavior while returning the explicit missing-statistics prerequisite.
Ruff, strict mypy (60 modules), formatting and the 3,917-declaration naming gate
pass. Final workspace Clippy with all targets/features and `-D warnings` passes
in 7m41s, including the final missing-report error mapping. No local acceptance
failure remains. This is targeted local evidence, not a claim that the full
workspace/process/container suite was rerun locally; exact-commit CI covers it.

Completed project test directories were removed (about 4.2 MiB), together with
one previously identified 108 KiB fixture. No other project's temporary files
were touched. The disposable PostgreSQL container and network were removed;
no production database or research history was modified.

The initial Rust check exposed a test attempting to access private executor
members. The fixture now uses the existing replay method; production visibility
and numerical authority boundaries were not widened. Runtime/lease deadlines
and numerical tolerances are unchanged. Existing process/crash regressions remain
part of the required exact-commit remote CI.

## Remote acceptance correction

Run `35587880648` on `8f30526` passed six jobs, including the complete unified
workspace gate (69m28s) and clean DaoCloud container gate (72m22s). The Rust job
was cancelled by its 60-minute total execution limit. Its check annotation says
"The job has exceeded the maximum execution time of 1h0m0s". The six new binding
cases, independent 2/4/8-process reconciliation writers and kill/restart cases
had passed, and further tests were still passing immediately before cancellation.
This is not a passing standalone Rust job or a closed phase.

Increase only that job's bounded total budget to 75 minutes, accounting for
dependency installation, compilation and the expanded real-worker regressions.
Keep every test, the serial test policy, application/lease deadlines and numerical
tolerances unchanged. The already-passing workspace/container jobs establish
that the full suite can complete; the corrected commit must still pass its own
remote gates. Reverting the workflow-only budget change restores the old CI
limit without changing executable behavior or any research/audit data.

## Rollback

Disable the optional reconciliation deployment, or restore a compatible previous
executable for v1 reads. Retain all reports, primary results, job receipts, trials
and audits; do not downgrade or delete v2 records. The old executable must deny
v2 policies. See the executable deployment and RPC procedure in
`docs/development/authorized-reconciliation.md`.
