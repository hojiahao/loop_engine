# ADR 0011: Numerical integrity primitives for US research

- Status: Accepted for the Phase 4 numerical boundary
- Date: 2026-09-09
- Owner: hojiahao

## Decision

Implement numerical primitives in `python/loop_research`, not in the Rust
orchestrator or Provider host. Declare NumPy as a research dependency and use
SciPy goldens plus bounded, deterministic Hypothesis cases for independent
verification. Keep Python 3.14.4 and the single root uv workspace/lockfile.

Adjusted Fisher-Pearson skew omits explicit NaN values and uses their actual
remaining count in both central moments and the adjustment. At least three
observations are required. An exactly constant sample has undefined skew and
returns NaN; the legacy zero convention is not carried into this new semantic
contract. It must not increase reported coverage. Compute centered, scaled
moments rather than subtracting large raw second/third moments. If subtracting
opposite finite extremes overflows, scale before centering.

The trailing reference kernel includes the current row, does not read future
rows, and requires explicit window/minimum-observation settings. Window width
is capped at 4,096. Missing current values do not discard valid earlier
observations in the window. This O(rows * window) primitive is a numerical
reference, not yet a panel evaluator or a throughput claim.

Simple portfolio returns are `NAV[t] / NAV[t-1] - 1`, with no invented first
return. Input NAV must already account for external cash flows. Require finite,
nonnegative observations and strictly positive denominators; a final zero NAV
is a total loss, while observations after insolvency require a separate
accounting policy. Reject overflow rather than emitting infinite results.

The correlation primitive derives both return paths itself and uses Pearson
correlation, never delta-NAV. It rejects unequal path lengths instead of
silently matching tails. Callers must resolve the same session index before
invocation; arrays alone do not prove calendar or PIT alignment. Constant or
insufficient return pairs produce NaN, not a fabricated zero correlation.
Masked, complex, string, Boolean, multidimensional and infinite observations
are rejected rather than silently coerced or losing missingness.

## Scope and remaining integration

These primitives begin migration of findings 4 and 8. Phase 4 is not complete:
protected capabilities, provenance invalidation, durable perturbation/failure
memory and unified readmission remain. Phase 6 must bind these semantics into
the operator registry and execute only the canonical AST. Phase 7 must resolve
aligned NAV paths and cash-flow accounting before calling the return primitive.
Legacy code and all stale A-share metrics remain untouched; no historical
performance, direction or admission is inherited by US candidates.

## References

- [SciPy adjusted Fisher-Pearson skew](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.skew.html)
- [NumPy Pearson correlation](https://numpy.org/doc/stable/reference/generated/numpy.corrcoef.html)
