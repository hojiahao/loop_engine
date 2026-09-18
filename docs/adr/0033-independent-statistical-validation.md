# ADR 0033: Independent Alphalens statistics and immutable reconciliation

- Status: Implementation in progress
- Owner: hojiahao

## Requirement

Phase 8 unit 1 must independently calculate the frozen cross-sectional statistics
of an actual reconstructed portfolio, explain differences, and preserve a
replayable result. Installing a package or echoing primary metrics is insufficient.
This unit grants neither job authority nor factor admission. Zipline accounting
and authenticated reconciliation remain the following delivery units.

## Decision

Alphalens Reloaded 0.4.6 successfully computes IC and grouped returns under
CPython 3.14.4, NumPy 2.5.2, SciPy 1.18.1 and pandas 2.3.3. Its declared
`pandas<3.0` constraint conflicts with the primary workspace's pandas 3.0.5.
Keep the main Python environment unchanged. Add `python/alphalens_validation`
with its own uv lock and an isolated ephemeral process, amending ADR 0003's
validator directory list and extending ADR 0005's compatibility exception.
The wrapper must never create a second project `.venv`. No primary numerical
module is importable through its declared dependency graph.

The primary administrative export first reconstructs and verifies the actual
portfolio/statistics receipt, then publishes raw frozen signal/eligibility/price
observations and exact primary comparisons as immutable CAS objects. The
independent worker consumes only those objects and the explicit calendar axis,
recomputes next-session raw open-to-close labels and frozen-direction ranks,
calls Alphalens for Rank IC and group means, and SciPy for Pearson IC and
group monotonicity. It independently implements the frozen equal-count grouping
with stable security-ID ties. It never imports primary labels, ranks or numerical
kernels as its calculation. Shared raw data and the export normalization remain
an explicit common dependency, not independent vendor certification.

Alphalens quantile turnover is a membership diagnostic, distinct from the
primary portfolio's executed-notional turnover. Preserve the complete session
axis, including unavailable dates; do not compress missing days or substitute
close-to-close returns. No demeaning or group adjustment is silently enabled.

Version the comparison profile and fixed numerical tolerances. Check all field
values and missingness; a mismatch produces inspectable differences. Too few
observations or unavailable required statistics cannot pass the gate. The final
unlabeled signal is expected and is not an observed forward return. Record the
actual validator source/dependency byte identity as well as versions. Replay
recalculates and verifies every output without writes or corruption repair.

## Acceptance and recovery

Use handwritten IC/group goldens, uneven groups, ties, reversed direction,
constant/missing values and calendar gaps. Exercise the actual two-process
workflow from frozen factor execution, mismatch detection, receipt corruption,
source drift, cancellation and bounds. Require offline locked execution and
prove the primary pandas version and single `.venv` remain unchanged. Extend
the existing workspace/CI gates; do not replace numerical tests with mocks.

Disable the new administrative commands to stop writers. Preserve inputs,
differences and receipts. No database migration, production-data entitlement,
holdout unlock or factor-library mutation is introduced.

## Primary sources

- [Alphalens Reloaded package](https://pypi.org/project/alphalens-reloaded/)
- [Alphalens source](https://github.com/stefan-jansen/alphalens-reloaded)
- [uv isolated execution](https://docs.astral.sh/uv/concepts/projects/run/)
