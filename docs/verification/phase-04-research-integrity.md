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
