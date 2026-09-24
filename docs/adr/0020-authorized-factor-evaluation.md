# ADR 0020: Authorized canonical factor evaluation

- Status: Accepted
- Date: 2026-09-12
- Owner: hojiahao

## Requirement

Phase 4 unit 5 must execute the canonical factor that the runtime authorized,
against its exact verified data view. A passing arithmetic helper or a new
unused interface is insufficient. Numerical values, coverage, operator semantics
and provenance must survive the actual worker/lease/result path together.

## Decision

Keep numerical work in the installed Python research package. Reuse the shared
canonical parser and immutable FactorSpec; do not evaluate expression strings,
Python code or a noncanonical submitted tree. A closed, versioned executable
operator registry binds each implementation to the exact semantic contract.
Unknown operators and unsupported semantic versions fail closed.

Implement the existing fourteen operator families with explicit new US-research
semantics. Rolling windows name both width and minimum valid observations in
the AST. Skew reuses ADR 0011's actual-count adjusted Fisher-Pearson kernel;
constant windows remain missing. Sample standard deviation uses effective
count and ddof=1. Ranking specifies ties and missing targets. Binary operators
require exactly aligned session/security axes. Floating-point arithmetic is
not declared associative merely to deduplicate differently parenthesized trees.
Retain all historical semantic variants; new behavior receives new identities.

The evaluator preserves immutable, strictly ordered session and security axes,
explicit eligibility and missingness. Coverage counts only eligible evaluation
cells, excluding warmup. No implicit fill, ticker join, calendar truncation or
direction reselection is allowed. Bound cells, AST size, window work and runtime;
invalid inputs, drift, cancellation and budget failure produce operational errors,
not invented factor metrics or successful admission.

Connect a fixed, deployment-selected numerical worker to ADR 0019's lease-bound
artifact preparation. Keep datasets outside RPC, verify actual mounted bytes,
and pin source/environment and registry identities before and after evaluation.
Persist outputs through the existing fenced completion and immutable artifact
mechanisms. Retry must recheck current authority and evidence; output existence
is not completion authority. No generic executable or container launcher is
exposed to the Agent.

This unit does not claim a complete portfolio backtester, market-data quality,
profitability or a live discovery loop. Those remain Phases 5-11. Protected
market execution stays disabled; synthetic acceptance cannot unlock real holdouts.

## Acceptance And Recovery

Require independent numerical goldens, missing/constant-window and no-lookahead
properties, canonicalization identity vectors, coverage/alignment failures,
actual installed-worker execution, lease expiry/cancellation, changed manifests,
restart and immutable replay. Preserve all three-language and OS-process gates.
Close this unit only after implementation, tests, documentation, one task commit,
push and remote CI pass together.

Rollback disables the new worker path. Do not rewrite old FactorSpec identities,
metrics, grants, receipts, manifests or audit history. Newly computed results
are bound to the new implementation and cannot be relabeled as old results.

Implementation `9c02629` is published. Local check/test/build/doctor/isolation
and all seven jobs in CI run `34747553685` pass. This accepts the bounded raw
factor workflow described above; later portfolio and production-data gates
remain unchanged.
