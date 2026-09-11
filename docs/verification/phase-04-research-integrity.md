# Phase 4 research-integrity verification

Status: in progress. These checkpoints do not enable production factor
discovery, authorize holdout data, or validate historical A-share performance.

## Numerical checkpoint

Commit `7e6352a` introduced the Python reference primitives under ADR 0011.
GitHub Actions run `34315225076` passed all seven jobs. The research package
contains 45 numerical tests plus one health test, including SciPy adjusted-skew
goldens, bounded Hypothesis properties, causal windows and NAV-return correlation.
The later storage regression was fixed in `33ea291`; all seven jobs in run
`34320106698` passed. Its separate evidence is in the Phase 3 verification record.

## Provenance comparison checkpoint

ADR 0012 reuses the six existing wire digests without a new Protobuf schema or
identity format. Rust, TypeScript and Python own immutable digest copies and
share 20 vectors in `tests/contracts/provenance_vectors.tsv`. The checks reject
every missing/incorrect-width digest and preserve all changed component names.
Recorded/frozen mismatch is an integrity failure before current-context lookup;
valid historical metrics are current, stale, or unresolved relative to an
explicit service-resolved context. Current-result gates reject stale/unresolved.
Existing development and holdout job validators reuse the same constructors.

Targeted local evidence:

- Rust: all 32 tests across nine protocol integration binaries pass, including
  four provenance tests for the 20 shared vectors and six-component
  absent/0/31/33/1,024-byte cases. The remaining binaries cover discovery,
  holdout, job, negotiation, research, runtime and wire-compatibility boundaries.
- Python: all 299 protocol-package tests pass, including 59 new provenance
  cases. Strict mypy also checks the new test module directly.
- TypeScript: all 113 protocol-package tests pass, including 54 new provenance
  cases. The new test module also passes strict TypeScript checking directly.
- Mutation checks exposed shallow freezing of empty TypeScript result arrays;
  both current and unresolved variants now freeze their nested arrays too.
- Host `just check` passed: unchanged generated/baseline artifacts and protocol
  boundaries, rustfmt, workspace Clippy with warnings denied, TypeScript
  format/lint/types, Python 3.14.4 environment checks, Ruff and mypy.

Commit `13aebad` is pushed. All seven jobs in GitHub Actions run `34324674436`
passed, including full workspace check/test/build/doctor, complete Rust storage
regressions and the clean DaoCloud container gate.

## Remaining gates

This is a shared metadata boundary, not complete finding-2 enforcement. Durable
result registration and read/export paths must independently resolve and verify
the original run, factor, sample, seed, source and immutable manifests, invoke
these checks, and prove invalidation across restart. Caller-supplied matching
digests cannot establish authority, correct execution or current availability.

Protected capabilities, perturbation/Sharpe history, failed-hash filtering,
unified readmission and numerical execution integration remain required by the
Phase 4 checklist. No paid data, LLM request, factor admission, research freeze,
holdout unlock or real backtest was performed by these checkpoints.

## Transactional result checkpoint

ADR 0013 and PostgreSQL migration 6 add immutable structured result registration
to the existing lease-fenced completion transaction. Current-result reads verify
the original job, result envelope and server-resolved current fingerprints.
Production resolution remains default-deny and migration 6 is not deployed to
the production database. No successful historical metrics are synthesized.

The new test binary contains 15 focused result-storage cases. The existing
process matrix additionally covers identical completion retries and distinct-key
revision races with 2/4/8 OS writers. The crash matrix adds stops after result
insertion, immediately before commit, and immediately after commit. All fault
hooks are compiled only into the library test binary.

The initial targeted test build succeeded. The subsequent full local build was
interrupted to prioritize the CI correction; it is not a recorded test pass.
Implementation `3e91027` is pushed. GitHub Actions run `34328644893` failed in
Rust, unified workspace and clean DaoCloud container checks. All three stopped
at the same `clippy::collapsible_if` in `store/backtest.rs`; TypeScript and all
three Python jobs passed. The follow-up collapses the nested condition into a
let-chain without changing the rejection rule or adding a lint exemption.

Correction `275e190` is pushed. Host workspace/all-target/all-feature Clippy
with `-D warnings` and rustfmt passed. In run `34330440769`, the Rust job passed
both lint and its complete regression suite; TypeScript and all three Python
jobs also passed. The other two jobs failed earlier during Rust component
downloads: unified bootstrap reported SJTUG connection timeouts and curl 56,
and the clean-container build reported SJTUG timeouts. DaoCloud OCI pulls
succeeded; these are transport failures, not evidence of a passing clean gate.

The follow-up uses one content-pinned downloader for host and container,
selects the official Rust endpoint on GitHub-hosted CI, and adds bounded fallback
without relaxing digest verification or changing DaoCloud images. Ten offline
fault tests pass locally and are now mandatory in `just check` and `just test`.
Host `just check`, shell syntax checks and both default/CI Compose transport
configuration checks pass. ADR 0002 records the transport policy. Full remote
acceptance of this follow-up is pending at commit time.

Commit `4581c53` is pushed and run `34333550936` passed all seven jobs. Rust
completed in 3m41s, unified workspace in 6m03s and the clean DaoCloud container
in 5m55s. This accepts the internal transactional result checkpoint. Production
resolvers, audited exports and the other Phase 4 integrations remain open.

## Audited metadata export checkpoint

ADR 0014 adds a separate metadata export command sharing the current-result
gate. It uses the existing immutable receipt table and one transactional audit
append; it does not write external files or grant artifact access. Every retry
revalidates read/export authority, the independent holdout policy, immutable
evidence and all six current-context fingerprints. Source results and terminal
job revisions remain unchanged. Explicit deadlines are bounded to 30 seconds;
cancellation before commit rolls back the receipt and audit together.

Local evidence on 2026-09-10:

- The 19 export tests pass, including stale-component replay, revoked authority,
  unavailable references, protected-job denial, actor spoofing, deadline and
  clock failures, corrupt receipts and audit rollback. The cancellation test
  waits for an actual PostgreSQL audit-insert barrier after receipt insertion,
  rather than assuming a sleep reached the transaction boundary.
- The existing 15 result-storage tests pass after sharing the current-result
  gate. Full `just check` passes, including workspace/all-target/all-feature
  Clippy with warnings denied, rustfmt, protocol compatibility, TypeScript and
  Python checks, and the ten downloader fault tests.
- The 2/4/8 OS-process matrix now includes same-key and distinct-key exports;
  the kill/restart matrix includes after-receipt, before-commit and after-commit
  stops. Their full execution and clean-container acceptance remain pending
  remote CI at this checkpoint; source coverage is not a passing test result.

No database table, service, dependency or production endpoint was added. Disable
a future export handler or restore the prior binary to roll back while retaining
all immutable receipts and audit events. Migration 6 remains undeployed to
production. ADR 0014 records the authority boundary, scope and recovery limits.

Implementation `214e170` is pushed and GitHub Actions run `34431407693` passed
all seven jobs. Rust completed in 4m06s, unified workspace in 6m19s and the clean
DaoCloud container in 6m37s. The full Rust suites exercise the added 2/4/8-process
export modes and receipt/commit kill points. This accepts the internal metadata
export checkpoint, not a complete Phase 4 or a production research workflow.

## Local NAV diagnostic checkpoint

The additive CLI consumes the existing NAV-return correlation kernel under
ADR 0011. Exact matching of full date sequences prevents comparing different
observation intervals, even when the arrays have equal lengths. Matching gaps
are not relabeled as daily returns. Raw file digests identify the bytes actually
parsed, not trusted market data or a six-component execution attestation.

Local evidence on 2026-09-10:

- Research package: 95 tests pass, including 49 new input, interval-alignment,
  undefined-result and actual-subprocess CLI cases. An independent SciPy golden
  verifies return correlation differs from delta-NAV correlation.
- The synthetic documented command executes successfully, reports seven return
  pairs and correlation `-0.7988091579053072`, and changes neither input file.
  This is test data, not US market performance.
- `just check` passes. Additional strict mypy checks of the source and new test
  module pass; only the test oracle's untyped SciPy import is explicitly exempt,
  with no production type-check exemption or new dependency.

No production data, holdout, database, artifact registry, factor admission or
backtest is accessed. There is no schema migration. Reverting this additive
command preserves existing `doctor` behavior and all historical state. Remote
acceptance remains pending at commit time; Phase 4 remains in progress.

NAV diagnostic implementation `8a39cdc` is pushed. Run `34434276768` passed
all seven jobs, including unified workspace commands and the clean DaoCloud
container gate. A local isolated environment also passed all 95 research tests,
proving the CLI does not depend on undeclared workspace packages.

## Development rejection memory checkpoint

ADR 0015 and migration 7 add an immutable, indexed projection of terminal
development backtest rejections. The existing direct/role submission and lease
acquisition paths consume deterministic failure memory under the ledger lock.
Completion commits projection, job, receipt and audit atomically. Holdout and
factor-evaluation jobs do not enter this memory; infrastructure failures are
never converted to factor rejection.

Local evidence on 2026-09-10:

- All 19 new PostgreSQL tests pass (21.94 seconds after compilation), covering
  both submission paths, queued work, restart/replay, all six fingerprints,
  factor/seed/snapshot changes, non-bypass via run/budget changes, rejection-code
  selection, infrastructure failure/cancellation, lease/clock failure, audit
  rollback, corruption, immutability and the pre-existing-history migration guard.
- An independent Node.js JSON/crypto calculation pins the v1 context-key golden
  to `ef69eb561404de69fe9e95e330e8b9f81982b9dc53736fabb9160035f16a9ffa`.
- `just check` passes, including workspace/all-target/all-feature Clippy with
  `-D warnings`, rustfmt, protocol and cross-language compatibility, TypeScript,
  Python and downloader checks. After the final golden case was added, the
  affected Rust test target passed Clippy again and was formatted with rustfmt.
- The first sandboxed database test attempt could not open a localhost socket
  (`Operation not permitted`); the authorized local-only rerun passed. This was
  not recorded as a database or research-behavior regression.
- The process matrix adds 2/4/8 OS writers for rejection completion/replay,
  completion revision races and rejected resubmissions. The kill matrix adds
  projection-insert, pre-commit and post-commit stops. These cases compile under
  the local gates; their execution and the full workspace/container gates are
  pending remote CI at this checkpoint's commit time, not claimed as local passes.

Manual review checked the filter is after authority resolution, new submission
and acquisition cannot bypass it, completion replay preserves immutable receipts,
and no source-job identity or holdout result is returned by the skip error.
The scope deliberately excludes in-flight deduplication and dynamic semantic/
correlation/policy evidence. Details and limitations:
`docs/development/failure-memory.md`.

No production database migration, credential use, paid data, model call, factor
admission or numerical backtest occurred. Migration 7 has not been deployed;
production remains at migrations 1-5. Before deployment the change can be
reverted normally. After deployment, retain immutable history and use a
schema-aware recovery build with writers disabled, not a destructive downgrade.
Phase 4 remains in progress; perturbation/Sharpe state, broader failed-hash
integration, unified readmission and authorized execution remain required.

Rejection-memory implementation `70b44c5` is pushed. Run `34441752040` passed
all seven jobs, including the complete 2/4/8-process and kill/restart matrices,
unified workspace commands and the clean DaoCloud container.

## Delivery Unit 1: Durable Perturbation

ADR 0016 adds an installed Python numerical worker and a Rust durable command.
The command resolves Sharpe from registered primary IS results, consumes the
existing indexed rejection memory, calculates outside the database lock and
commits RNG/history, receipt and audit together. Migration 8 is local-only;
the production database remains untouched. There is no real market backtest,
external model request, factor admission or holdout unlock in this delivery.

Local evidence on 2026-09-10:

- The 28 new Python cases pass, including real Protobuf subprocess round trips,
  a fixed cold-start RNG golden, analytical Sharpe-gradient values, restart
  equivalence, bounded Hypothesis proposals and malformed-envelope rejection.
- The 21 PostgreSQL integration cases pass in 32.07 seconds, including the real
  Python worker, durable history/RNG, retry/CAS conflicts, current provenance,
  cancellation/deadline/clock failures, audit rollback, SQL guards, corruption,
  holdout-source denial and a rejection committed during numerical computation.
- The library suite passes (4 tests plus the explicitly invoked crash helper),
  including worker error classification and the kill/restart matrix. The new
  fault points are after state/receipt insertion, before commit and after commit.
- The full independent-process matrix passes in 181.93 seconds. The two new
  modes cover same-key replay and distinct-key CAS races at 2/4/8 writers. Every
  case commits one history observation, one proposal, one raw RNG draw and one
  receipt, with a verified audit chain after all processes exit.
- A first 17-case database run passed 16 cases but one real worker hit its
  10-second limit while an overlapping Clippy compilation competed for resources.
  This is retained as failed evidence, not silently relabeled as a pass. A
  serialized rerun passed all 21 current cases without relaxing the deadline.
- An offline isolated Python run could not fetch the newly declared protocol
  dependency's uncached grpcio wheel. The locked online rerun was interrupted,
  and was not counted as a pass. After the locked wheel was cached, the final
  offline isolated research run passed all 151 cases in 29.51 seconds.
- The first final workspace check correctly refused the old wire-fixture
  descriptor digest after the additive protocol extension. Regeneration changes
  only that digest in `wire_fixtures.json`; the binary fixtures and compatibility
  baseline remain unchanged. The subsequent complete `just check` passed:
  protocol boundaries/compatibility, cross-language fixtures, rustfmt, workspace
  Clippy with warnings denied, TypeScript checks, Ruff, mypy and Python 3.14.4
  single-root-environment verification.

The local test container and its disposable database were removed after tests.
Commit publication and remote CI are pending at this record's commit time;
acceptance requires all seven jobs on this delivery commit, not an earlier run.
Commands and rollback: `docs/development/perturbation.md`. Trusted production
family/result manifests and actual authenticated data isolation remain required
by delivery units 3 and 4; numerical execution integration is unit 5.

### Remote Fixture Capacity Correction

Implementation `8095a9a` was pushed. Run `34458505546` failed three of seven
jobs: Rust, unified workspace and the DaoCloud container. Each failed with
PostgreSQL SQLSTATE `53100`, `No space left on device`, in the disposable
database. Four language jobs passed. The 512 MiB tmpfs filled within one fresh
full-suite run, not because of production data or a missing between-run cleanup.
This failed run does not accept delivery unit 1.

Both test-only Compose definitions now cap PGDATA at 1 GiB and container memory
at 1280 MiB. The existing per-run schema isolation and teardown remain intact;
managed gates report filesystem usage before disposal, including on failure.
The Rust CI job now explicitly tears down its fixture as well. No runtime
algorithm, worker deadline, test assertion, migration or production setting is
relaxed. Reverting the correction restores the previous test-only resource
budget and commands, without affecting immutable research history. Verification
and a new remote run are required before accepting the delivery.

Both definitions pass `docker compose config --format json` checks of the tmpfs
mount and the normalized 1342177280-byte memory limit. All three changed shell
scripts pass `bash -n`; `git diff --check` passes. A local full-suite compilation
was interrupted by the conversation restart and is not recorded as a pass. The
process was confirmed stopped before restarting `just test` against a fresh
fixture. That full rerun and the corrected commit's CI are still pending at
publication; the correction does not itself close Phase 4 or delivery unit 1.

### Rust Job Cache Correction

Capacity correction `5dabadd` is pushed. In run `34463208111`, the Rust
format/Clippy/test steps and database cleanup all passed. The measured database
usage was 548596 KiB (53% of the new 1 GiB bound), exceeding the old 512 MiB
bound. The unified workspace job also passed. The Rust job nevertheless failed
in setup-uv's post-job hook: it looked for its default temporary cache, while
`scripts/uv.sh` had populated `.tools/uv-cache`. This is a cache-path integration
error, not a passing CI job or a numerical-test failure.

The Rust setup-uv step now explicitly selects the same workspace cache path as
the other Python-enabled jobs. Caching and all verification steps remain enabled;
no empty placeholder directory or ignored post-job failure hides the mismatch.
This workflow-only correction leaves runtime code and database history unchanged.

The completed run has six successful jobs, including the clean DaoCloud container;
only Rust's cache post-hook failed. A structured YAML check verifies all five
setup-uv jobs enable caching at the same explicit workspace path, and
`bash scripts/uv.sh cache dir` confirms that actual location. `git diff --check`
also passes. Remote acceptance of this correction is pending at commit time.

The local capacity rerun completed compilation and several PostgreSQL suites,
then was interrupted again. After confirming no test process remained, its
273596 KiB partial-use reading was recorded and the disposable service removed.
That partial run is not a full-suite pass. The completed remote Rust test step,
unified workspace gate and clean-container gate above provide the full-suite
capacity evidence; no further local recompilation is required for two YAML
cache inputs and their documentation.

Correction `62dbcb5` is pushed. Run `34465141418` passed all seven jobs,
including the Rust cache post-hook, full workspace commands and clean DaoCloud
container. This accepts delivery unit 1, not Phase 4 as a whole. Production
reference resolution and authenticated research execution remain denied.

## Delivery Unit 2: Unified Factor Admission

ADR 0017 implements one admission/readmission command over registered primary
IS results. Both paths check current provenance, coverage, deterministic gates,
completed semantic review and the reviewed active library. A human exception
can waive only an actual semantic rejection, with independent authorization and
explicit audit. Replacement retirements retain lifetime counters and commit in
the same transaction as admission, immutable receipt and audit.

Migration 9 adds the immutable trial index and revision-fenced factor projection.
The existing shared job insertion path registers factor-evaluation/development
backtest trials atomically, including jobs that later fail or are cancelled.
Trials retain actual execution attempts, not just successful admission votes.
Protected jobs are excluded. Existing research jobs require a separately verified
import; the migration refuses to invent trial history from opaque Protobuf.

Local evidence on 2026-09-11:

- The final 25 PostgreSQL integration cases pass in 27.06 seconds. They cover
  restart/replay, exact coverage boundaries, repeated rejection/readmission,
  explicit semantic override and revocation, non-overridable machine gates,
  missing evidence, replacement and lifetime retirement accounting, stale
  library and all six provenance components, authority/default denial,
  revision/key conflicts, deadlines/clock regression, cancellation, corruption,
  immutable SQL guards, audit rollback, trial pagination, executed rejection,
  infrastructure failure, migration refusal and protected-source exclusion.
- The library gate passes six tests in 40.64 seconds, including the two new
  digest/integer goldens and the kill/restart matrix. Its subprocess helper is
  explicitly invoked by that matrix, not silently omitted. New fault points
  stop after state/receipt insertion, before commit and after commit; recovery
  preserves a single revision, admission and receipt with a valid audit chain.
- The independent-process matrix passes in 189.32 seconds. New same-key replay
  and distinct-key CAS races run at 2/4/8 OS writers; every case retains exactly
  one admission, no spurious retirement and one immutable decision receipt.
- Final `just check` passes: shell/downloader checks, protocol compatibility
  and boundaries, cross-language fixtures, workspace rustfmt/Clippy with
  `-D warnings`, TypeScript formatting/lint/types, Python Ruff/mypy and single
  root Python 3.14.4 environment verification. This covers the final test code.
- Independent Node.js crypto pins the empty active-library digest to
  `1519092edd04a3f68553510913c546c1c557c7e3f0e1bff681f1694275b0a25a`.
  A separate integer test exercises coverage at u64::MAX without overflow or
  floating-point rounding. Execution is recorded with the library gate below.
- Earlier 15-case runs exposed a PL/pgSQL CASE-parenthesization error, then nine
  strict audit-payload ordering failures. Both were fixed in this unreleased
  delivery. The ordered serializer uses the existing canonical audit validator;
  no assertion or canonical format was relaxed. Those runs are failures, not
  acceptance evidence; the final 25-case run above is the passing result.

Compilation completed before PostgreSQL tests to avoid memory contention on the
small development host. After confirming the compiler had exited, 1.1 GiB of
rebuildable Rust incremental cache was removed. Source, installed dependencies,
test executables and research/audit history were retained. The first passing
25-case run used 90340 KiB of its disposable 1 GiB PostgreSQL filesystem.
The subsequent process/fault run used 280740 KiB before its container was
removed. The full workspace gate enables additional protocol features; thirteen
redundant single-package test executables (1.2 GiB) were removed after verifying
the active workspace compilation used different cache identities. No active
compiler inputs or workspace test executables were deleted. Targeted commands
now retain the workspace feature selection to avoid recreating both caches.

Final `just test` passes: Rust 315 tests plus two explicitly exercised subprocess
helpers, TypeScript 114, Python research 151, Python protocol 299, and legacy
216 passed / 1 skipped. The legacy suite also reports 11 existing NumPy
degrees-of-freedom warnings. The placeholder Web package has no test files;
this is not UI acceptance. The full workspace process matrix passes again in
199.79 seconds; the new 25-case admission suite passes again in 24.94 seconds.
The fresh full-suite database used 666876 KiB (64%) before automatic disposal.
No test deadline, canonical assertion or database capacity was relaxed.

At the owner's request, after all test processes exited, `cargo clean` removed
16192 generated files (11.6 GiB reported). Project-owned Python test/type caches,
bytecode, frontend/package build outputs and this run's confirmed pytest-0/1
directories under `/tmp/pytest-of-root` were also removed. Installed toolchains,
dependency environments/download caches, source, verification documents, secrets
and research/history backups were retained. Other projects' `/tmp` content was
not modified. Filesystem usage fell from 97% to 67%, with approximately 13 GiB
available. Subsequent Rust builds must recreate their generated artifacts.

Commit publication and remote CI are pending at this record's commit time.
The fixture evidence is explicitly
fabricated IS metadata over real jobs/transactions, not actual market evaluation.
Production `BacktestPolicy` still denies admission and override resolution; no
external admission RPC, production migration, factor admission, model request,
paid data download or holdout unlock has occurred. Units 3-5 supply trusted
manifests, runtime identities/isolation and authorized numerical execution.

Manual review checks shared ordinary/readmission gates, non-overridable data
integrity, historical replay without double retirement, SQL/source binding and
default denial. The deployment bundle now grants UPDATE on the two mutable
perturbation/factor projections, without granting UPDATE/DELETE on immutable
history. It was generated for inspection only, not deployed. Disable writers
for rollback and retain migration 9, trials, states, receipts and audit; after
deployment use a schema-aware compatibility build or forward fix. See
`docs/development/factor-admission.md`.
