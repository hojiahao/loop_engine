# ADR 0031: Frozen statistics and complete declared experiment families

- Status: Accepted; commit a512220, exact-commit CI 35180788482 passed all seven jobs
- Date: 2026-09-17
- Owner: hojiahao

## Requirement

Phase 7 unit 3 adds inspectable statistics to the actual portfolio workflow:
IC/Rank IC, groups, turnover, drawdown, exposures, uncertainty, FDR, DSR and
CSCV/PBO. Imported scores and a selected list of profitable trials are not
sufficient evidence. Unit 4 still owns authenticated durable completion and
the global trial registry; this administrative workflow grants no admission.

## Decision

Reuse portfolio reconstruction and its live provenance/file guards. The opt-in
`daily-statistics.1` evaluation policy freezes group count, minimum cross section,
minimum time observations, Bartlett HAC lag and even CSCV block count. Old
coverage-only policies keep their original meaning and cannot produce statistics.
All outcomes identify their evidence; undefined statistics have explicit reasons.
No service, database table, RPC or numerical dependency is added.

Signals at close t are evaluated against raw open-to-close returns on t+1.
This is an execution-aligned intraday diagnostic, not an overnight/total-return
label or a group trading simulation. Only signals eligible and finite at t enter
the universe. An absent next open/close invalidates the whole cross section;
do not silently drop a delisting or halted security. Apply only the frozen
direction. Pearson IC and average-tie-rank IC use the same observations. Groups
are equal-count, ordered worst to best with stable security-ID ties. Report
arithmetic group returns and best-minus-worst spread, without calling them
costed executable portfolios. The last signal has no forward label.

Portfolio statistics use exact ledger NAV ratios. One-way turnover is half of
absolute executed notional divided by previous NAV, including opening trades;
drawdown uses the running NAV peak including initial capital. Signed market
value/NAV gives net exposure, absolute values give gross exposure. Industry,
signed weighted beta and signed weighted log USD market-cap exposures require
the original causal exposure panel; missing held-security data is unavailable.
Cash and unsettled action claims are not assigned fictional equity exposures.

Mean uncertainty uses a Bartlett Newey-West covariance of the intercept with
the n/(n-1) correction, asymptotic normal two-sided p-values and 95% intervals.
No missing-date compression is permitted for HAC. The Sharpe convention is
daily sample mean/sample standard deviation against an explicit zero benchmark;
sqrt(252) scaling is descriptive, not an autocorrelation correction. Missing,
short or degenerate series do not produce infinite t values or Sharpe ratios.

Multiple testing is restricted to an immutable, predeclared finite family.
Its plan commits to every trial's expression/direction, eight non-evaluation
policies, data/work context and execution tape. Each evaluation policy pins the
plan hash and its trial ID before evaluation. Completion must cover exactly
the plan, including failures; successful receipts are actually reconstructed.
The family must share policy, sample, source/environment, calendar and source
data context. This proves completeness of that declared family, not completeness
of a person's prior/adaptive searches. Reports explicitly retain that boundary;
unit 4 must bind the trusted global trial registry before admission.

Benjamini-Yekutieli adjusts the entire family's two-sided NAV-mean HAC p-values
(5% threshold); failed/undefined trials contribute p=1 and cannot be discoveries.
Its dependence robustness does not cure invalid marginal p-values, post-hoc
family selection or nonstationarity. DSR and CSCV require every planned trial's
complete synchronous return series. DSR uses sample Sharpe variance, biased
central skew/Pearson kurtosis and all declared trials as the independent-count
assumption; report that assumption rather than certify independence. CSCV uses
equal contiguous blocks, all half-block combinations, average ranks, and the
first plan-order winner for exact ties. Report PBO as fraction logit <= 0. A
nondivisible sample, any degenerate split or insufficient sample makes it
unavailable; never trim dates or discard bad splits. CSCV is internal selection
diagnostics, not locked holdout evaluation or causal retraining of strategies.

## Acceptance and rollback

Require independent numerical goldens, tied/constant/missing/short-series cases,
complete-family and failure-count checks, actual installed CLI execution,
byte-identical read-only reconstruction, corruption and deadline failures.
Publish CSV/JSON artifacts before the receipt, preserving existing CAS semantics.
The 180-second cooperative budget also bounds family work and split enumeration.
No production credentials, holdout access or final research conclusions are used.

Disable the new CLI commands or revert this task commit to roll back. Keep all
published plans, receipts, statistics and previous audit records. No destructive
schema migration is needed; source changes invalidate current reconstruction.

## Primary references

- [Statsmodels HAC covariance](https://www.statsmodels.org/stable/generated/statsmodels.stats.sandwich_covariance.cov_hac.html)
- [Statsmodels multiple-testing methods](https://www.statsmodels.org/stable/generated/statsmodels.stats.multitest.multipletests.html)
- [Bailey and Lopez de Prado, Deflated Sharpe Ratio](https://www.davidhbailey.com/dhbpapers/deflated-sharpe.pdf)
- [Bailey et al., Probability of Backtest Overfitting](https://www.davidhbailey.com/dhbpapers/backtest-prob.pdf)
