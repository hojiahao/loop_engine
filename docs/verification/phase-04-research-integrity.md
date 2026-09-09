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

Full workspace/static and clean-container acceptance must refer to the CI checks
on the provenance implementation commit, not the previous storage commit.

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
