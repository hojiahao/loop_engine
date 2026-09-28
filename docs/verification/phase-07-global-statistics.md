# Phase 7 unit 5: authenticated global statistics

Status: accepted. Commit `e325368` is pushed; exact-commit CI `35569521771`
passes all seven jobs. This closes Phase 7's development implementation scope.
Decision: ADR 0036. Workflow: `docs/development/global-statistics.md`.

## Final local acceptance, 2026-09-21

All six actual Rust/mTLS/PostgreSQL cases pass in 452.84 seconds: incremental
registration of two numerically computed strategies, available PBO, restart and
idempotent replay, current reads, pending-trial diagnostics, operator denial,
imported-result denial, expired leases and atomic snapshot fencing. Changing
only a trial state/revision after calculation rejects completion without changing
the report job or appending a success audit.

The combined numerical regression run passes 78 cases, and the final expanded
worker suite passes all eight cases (including five additional negative cases).
All 306 Python protocol cases, generated-binding/wire/compatibility/boundary
checks, TypeScript checks/tests, Ruff, strict mypy, Rust formatting and the
3,903-declaration naming check pass. Final workspace Clippy with all targets,
all features and warnings denied passes in 6m26s.

Test directories created for the Python/protocol runs were removed; completed
Rust fixtures left no `loop-manifests-*` directory. One identified obsolete test
executable was removed, recovering about 172 MiB of reproducible build output.
The disposable PostgreSQL test container and its network were removed.
Full remote workspace, process/crash and DaoCloud-container gates pass. Rust
completed in 52m13s, unified workspace in 61m22s and the clean DaoCloud container
in 54m04s. These are complete CI job durations, not a single factor's latency.

## Delivered code under verification

The runtime now captures a whole-registry revision/state snapshot in addition
to the existing conservative job/attempt ledger. An optional fixed Python
supervisor reconstructs every registered successful development portfolio and
builds a common-date strategy matrix. The existing Report job lifecycle
registers the immutable result with the source checks, lease, receipt and audit
in one transaction. Execute/read RPCs expose no metrics, paths or subset fields.
No new service, table or numerical dependency is introduced.

The report distinguishes complete data from usable statistical estimates.
Failed/pending/retried work, absent continuations, incompatible contexts and
duplicate configurations are accounted for explicitly. Operational duplicates
are not independent strategy columns. Existing DSR/CSCV assumptions remain
visible, and supported development data never opens production admission.

## Local evidence so far

- All 25 new population/matrix/policy tests pass, covering numerical-kernel agreement,
  canonical membership, failed/retried work, duplicate strategies, date/context
  mismatch, bounds and deadline propagation.
- The 26 existing statistical-kernel tests passed during the first combined run;
  that run initially exposed a strict-JSON fixture construction error in the new
  tests, which was corrected without weakening model validation.
- All three actual two-direction Python workflow tests pass: complete matrix,
  immutable replay, corrupt-primary denial and changed-population denial.
  The first attempt under concurrent Rust compilation exhausted native build
  verification's existing deadline. The later serial run passes with unchanged
  production limits. Shared broker-published read-only market objects are reused
  instead of being rewritten by the second test strategy.
- Ruff and strict mypy pass (60 source modules). Naming and Rust format gates
  pass. Workspace Clippy with `--all-targets --all-features -- -D warnings`
  passes after adding the two missing imports in the new transaction test.
- The final numerical implementation passes 78 combined Python regressions,
  including incremental strategy registration, immutable replay, population
  diagnostics, statistical kernels and the existing portfolio worker. The final
  expanded worker suite also passes all eight cases (120.92 seconds), including
  incomplete-population corruption and four historical identity/attempt changes.
- TypeScript format, lint, type checks and tests pass.
- All 306 Python protocol tests pass against the regenerated bindings.
- Generated Rust, TypeScript and Python protocol bindings and wire fixtures
  have been refreshed. Full protocol generation, wire-producer consistency,
  backward compatibility and service-boundary checks pass.

Publication and exact-commit CI are complete. The separate Phase 8, licensed-data,
economic acceptance and semantic-review gates remain required.

The first six authenticated cases exposed a fixture/config mismatch: policy
revision `v1` is invalid under the pre-existing positive-integer wire contract.
The fixture, Python policy model, Rust policy validation and documented example
now use revision `1`; invalid/zero/leading-zero/overflow revisions have explicit
Python regressions. The protocol contract was not relaxed. A subsequent run passed
four cases and exposed an incorrect expired-lease expectation plus a read-budget
bug: numerical runtime was included in the 10-second file-read budget. Lease
errors retain their existing typed precondition mapping; separate bounded input
and output reads now sit beneath the unchanged operation deadline.

The final flow verifies registered source metadata first and reconstructs each
source numerically once. It can refresh global statistics after later trials
without rewriting earlier primary reports: historical trial identities must
remain in the registry with nondecreasing attempt counts. Ordinary primary
reads still reject stale search counts. A real transaction regression changes
only a trial's state/revision after calculation and requires atomic rejection.

The next authenticated run passes five of six cases, including atomic snapshot
fencing. The two-strategy report registers successfully but its fixture contains
15 actual returns after warmup and initial NAV, so equal-block CSCV correctly
reports `unequal_block_lengths`. The synthetic shared integration sample now
includes the next valid session, providing 16 returns without trimming real
observations or changing a statistical threshold. Replay/current-read acceptance
has passed with that complete sample before publication.

## Recovery

Disable the optional statistical reporter to stop new writers. Roll back code
without deleting existing policies, trial history, primary results, reports or
audits. No schema migration or destructive down-migration is necessary.
