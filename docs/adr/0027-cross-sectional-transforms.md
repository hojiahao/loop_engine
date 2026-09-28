# ADR 0027: Frozen cross-sectional factor transformations

- Status: Accepted; published and verified
- Date: 2026-09-15
- Owner: hojiahao

## Requirement

Phase 6 unit 2 adds winsorization, standardization and industry/size/beta
neutralization to actual authorized factor evaluation. It must use the frozen
FactorSpec policy identities, causal exposure observations and independent
numerical goldens. Array helpers alone do not close this unit.

## Decision

Reuse the existing panel builder, data broker, worker RPC and completion path.
A version-2 panel carries the actual preprocessing/neutralization policy
documents and, when required, one additional dense exposure CSV artifact.
The builder resolves private source evidence; the broker only exposes the
derived authorized grid. No new service, database table, dependency or RPC is
needed. The runtime verifies policy/artifact identities, never statistical fits.

Version-1 panels retain raw evaluation with empty transformation policies.
Nonempty unsupported policies must fail, rather than being silently ignored.
Version-2 evaluation requires its explicit engine and artifact versions. Both
the worker and runtime compare policies with the FactorSpec and frozen context.
New outputs use a version-2 result manifest, preserve raw coverage and report
transformation outcomes. Historical identities and receipts are not rewritten.

The numerical profile is equal-weight, session-local: select eligible finite
factor values with all requested exposures, clip symmetric linear-quantile
tails, fit the declared neutralization, then optionally standardize residuals
with sample standard deviation (ddof=1). Quantile tail basis points and minimum
observations are explicit frozen integers. No cross-date fit or implicit fill
occurs. Direction stays fixed and is not reselected or reapplied here.

OLS always includes an intercept. Industry levels are ordered, with the first
level omitted as reference; positive USD market capitalization is log-transformed,
and supplied beta is continuous. Continuous columns are centered/scaled before
the fit. The profile fixes a relative SVD cutoff of 1e-12 and at most 64 design
columns. Missing required exposures exclude cells without changing the eligible
coverage denominator. Too few observations, rank deficiency and effectively
constant standardized results produce missing values with explicit outcomes.
Unexpected linear-algebra, nonfinite arithmetic or budget errors fail the job.

Exposure observations have stable security/session identities, source digests
and effective/known/ingested timestamps. Latest visible revisions are selected
at the panel's decision, never backfilled. Automatic vendor-grade industry,
market-cap or beta estimation is outside this unit: supplied exposure evidence
remains an explicit data-owner declaration and keeps development quality.

## Acceptance and recovery

Use independent hand/SciPy goldens for clipping, z-scores, industry demeaning and
multi-exposure residuals; test orthogonality, constants, sparse/rank-deficient
designs, extreme finite values, ordered axes, late revisions and immutable replay.
Exercise the installed builder and real TLS/PostgreSQL/Python worker, including
policy mismatch, altered exposures, stale provenance and cancellation failures.
Run the existing repository gates, then publish one task commit and verify CI.

Rollback disables new version-2 writers/evaluation and retains the raw version-1
path. Preserve completed source captures, panels, factor identities, job receipts
and audit events. No destructive database migration, paid request, production
data admission or holdout unlock is introduced.

Implementation `e881d3a` and assertion correction `a1ec5e3` are published.
All seven jobs in CI run `34947453231` pass, including full Rust, unified
workspace and DaoCloud container gates. The correction changes only a negative
test's expected transport error code. Phase 6 unit 3 remains separate.
