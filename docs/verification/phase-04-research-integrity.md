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
