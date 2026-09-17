# Phase 7 primary portfolio backtest verification

## Unit 3: statistics and multiple testing (`in_progress`)

Requirement and exact assumptions: ADR 0031. The installed `statistics-run`,
`statistics-validate` and plan-binding command use genuine numerical/portfolio
reconstruction. Four cohesive numerical/contract/workflow modules reuse the
existing CAS, evaluator and accounting. No dependency, service, database table
or protected-data access is added.

| Requirement | Executable acceptance |
| --- | --- |
| Intercept Newey–West covariance and normal uncertainty | `test_hac_golden`, independent SciPy SEM/t-stat comparison |
| Average ranks, IC, groups, frozen direction and future-label missingness | `test_portfolio_statistics.py` goldens and causal eligibility cases |
| BY-FDR on the full family | Hand-calculated adjusted p-values and permutation/monotonicity properties |
| DSR moments and declared-count assumption | Independent SciPy skew/Pearson-kurtosis and equation checks |
| Exhaustive CSCV and deterministic ties | Six hand-derived splits; missing/unequal/constant/short/budget negative cases |
| Actual portfolio statistics with complete synchronous trials | `test_actual_statistics`, 17 XNYS sessions and two actual portfolio replays |
| Failures remain in counts; no missing/duplicate/substituted trial | `test_failure_count`, family integrity and frozen-policy mismatch tests |
| NAV ratio, drawdown, turnover and signed risk exposures | Small ledger/drawdown goldens and long/short exposure weights |
| Repeatability, provenance, corruption, cancellation and deadline | Installed CLI round trip, read-only bytes/mtime checks, Decimal-context and interruption tests |

The initial new numerical/workflow suite passed 41 tests in 199.88 seconds.
A later regression run was stopped after a newly added constant-series guard
referenced the wrong local variable; that run is not counted as passing. The
guard was corrected, and explicit decimal constants, complete policy comparison
and isolated Decimal contexts were added. The numerical/cross-section suite then
passed **45 tests in 3.55 seconds**. Ruff lint/format (90 files), strict mypy
(53 source files), and the 3,452-declaration Python/Rust/Shell naming gate pass.
Final combined regression and exact-commit remote CI are recorded below when
completed; this note alone does not close the delivery gate.

Final affected-suite run: **187 passed, 1 setup error in 713.38 seconds**. All
65 new statistics cases passed. The error was the existing
`test_backtest_workflow.py::test_protected_window` fixture hitting
`worker build verification timed out` while hashing the native environment;
it did not reach protected-window execution. The same case passed alone in
**6.42 seconds**, with no source, limit, assertion or skip changes. Do not describe
the first run as completely passing. All 188 cases have passing local evidence
across these two runs; exact-commit CI must also pass the complete gates.

```sh
./scripts/uv-research.sh run --locked --offline --no-sync pytest \
  tests/test_statistics_kernels.py tests/test_statistics_workflow.py \
  tests/test_portfolio_statistics.py tests/test_backtest_workflow.py \
  tests/test_portfolio.py tests/test_market_workflow.py tests/test_market_portfolio.py \
  tests/test_factor_worker.py tests/test_transform_pipeline.py

./scripts/uv-research.sh run --locked --offline --no-sync pytest \
  tests/test_backtest_workflow.py::test_protected_window
```

Ruff lint and format (90 files), strict mypy (53 source files), the handwritten
naming gate and Rust formatting pass. Financial paths preserve their existing
deadlines and numeric bounds. The seven-job remote workflow supplies the full
workspace, Rust/Clippy and clean-container gates; its exact commit/run receipt
will accompany publication and be pinned in the next task's checklist update.
Temporary local fixtures and XML are disposable after this evidence is recorded.

The family receipt proves completeness only of its declared batch. Plan timing,
failure statements and the global search history are not authenticated here;
they must be bound to the durable registry in unit 4. Every report remains
`production_eligible=false`. The statistical values are synthetic acceptance
evidence, not market profitability or independent out-of-sample conclusions.

Rollback disables the new CLI writers or reverts this task's implementation,
retaining all CAS plans, ledgers, statistics, receipts and audit history. There
is no schema migration or destructive recovery action.

## Unit 1: next-session ledger (`complete`)

Commit `cf2750dd4aaacabccb258dfd6815cf034a1c452d` is pushed. GitHub Actions run
[`35065117368`](https://github.com/hojiahao/loop_engine/actions/runs/35065117368)
passes all seven jobs, including isolated Python research with its WRDS fixture,
Rust, unified workspace and the clean DaoCloud development container. This closes
unit 1, not Phase 7 or a production-data validation gate. The local history below
is retained as evidence; the exact-commit CI supersedes its pending status.

Requirement and scope: ADR 0029. Implementation provides bounded administrative
`backtest-run` and read-only `backtest-validate`, using actual recomputed factor
evidence, all frozen policy identities and explicit raw execution observations.
No service, database table, dependency or production-data permission is added.

| Requirement | Executable evidence |
| --- | --- |
| Cash/holdings/NAV reconciliation with opening gaps | `test_hand_accounting` |
| Successive NAV returns, absent first return | `test_ratios_first` |
| Fixed-share next-session orders, causal prefixes | `test_next_open`, `test_future_signals` |
| Chronological sale funding | `test_late_sale` |
| Commission/spread without double charging | `test_commission_spread` |
| Missing observations and explicit terminal holdings | `test_unfilled_open`, missing-mark tests, `test_final_positions` |
| Deterministic ties, direction, decimal context and conservation | Direction/tie/context tests and `test_flat_frictionless` |
| Actual numerical source and installed CLI | `test_evaluation_ledger`, `test_installed_cli` |
| Fail-closed evidence, policy, provenance, sample, clocks and corruption | `test_backtest_workflow.py` negative cases |

Phase 6 closeout `4ea0316` is pushed and passes CI run `35060368321`; its
underlying implementation remains `b792601` with run `35058329373`.
Initial ledger-only acceptance: 18 passed. Implementation acceptance is recorded
below; publication and the associated exact-commit CI still gate delivery.

The first integration run passed 59 cases in 164.10 seconds. The expanded full
research run, concurrent with local Clippy, produced 772 passed, 7 skipped,
1 failed and 5 setup errors in 966.85 seconds. Five errors were explicit
native-environment byte-verification timeouts; the installed CLI replay returned
exit 2. The seven skipped cases require the optional disposable WRDS PostgreSQL
fixture, which was not started for this local Python run. CI runs that fixture.

All six failed/error cases subsequently passed serially in 43.26 seconds.
The extraction now carries the encoded value CSV with the computed result,
preserving the original factor worker's pre/post verification points without an
extra full native scan. Portfolio release still verifies current build and all
inputs before the final receipt; incomplete CAS objects grant no completion.
Production timeout, byte/work limits and corruption checks are unchanged.

Ruff lint/format and strict mypy pass (46 source files); Rust formatting and
workspace Clippy with `-D warnings` pass. Clippy completed in 9m05s on this host.
The final affected-suite run passes **77 tests in 281.46 seconds**, including
all failed/error cases, actual raw/v2 factor computation, installed CLI replay,
numerical goldens and full interruption/recovery behavior. Command:

```sh
./scripts/uv-research.sh run --locked --offline --no-sync pytest \
  tests/test_backtest_workflow.py tests/test_factor_worker.py \
  tests/test_portfolio.py tests/test_transform_pipeline.py
```

The complete exact-commit workspace/container run is delegated to the existing
seven-job GitHub workflow after publication. Do not treat the resource-contended
local full run as fully passing, or the affected-suite result as a complete
production backtester. Publication/CI evidence accompanies this task commit and
is pinned in the next phase-task update after its check run completes.

At unit 1 closure, remaining Phase 7 units were action/financing/borrow/capacity accounting and PIT
execution inputs; statistics/multiple-testing; authorized runtime completion,
current reads/exports and admission. Independent validation remains Phase 8.
Licensed historical production-data coverage remains deferred by the owner.

Rollback: disable new portfolio writers or revert the unit's code commit,
preserving immutable inputs/results/receipts. No schema migration or deletion of
research/audit history is needed. Temporary test directories may be removed
after evidence is recorded; actual market captures and research artifacts remain.

## Unit 2: PIT actions, financing and capacity (`complete`)

Commit `4953dc90f33cd86d38df1b9e2019a958631b487b` is pushed. GitHub Actions run
[`35082001411`](https://github.com/hojiahao/loop_engine/actions/runs/35082001411)
passed all seven jobs, including unified workspace and the clean DaoCloud
container. This supersedes the pre-publication pending notes below and closes
unit 2 only.

Requirement and exact model: ADR 0030. Adds `pit-actions-long-short.1` through the
existing installed CLI, with a source-backed v2 tape and explicitly frozen new
policies. No extra service, database table, dependency or paid access is added.
Prior v1 algorithms remain available. Byte integrity and declared public clocks
do not certify historical source coverage; all receipts remain development-only.

| Requirement | Executable acceptance |
| --- | --- |
| Share conversion, pending targets, long/short fractional cash-in-lieu | Split and reverse-split goldens in `test_market_portfolio.py` |
| Entitlement versus cash payment; lender liabilities | Dividend, short-dividend and unpaid-payable-reserve goldens |
| Explicit cash/zero delisting consideration and permanent retirement | Long and short delisting goldens |
| Borrow limits, actual recalls and unavailable markets | Availability/recall/held-mark tests; unfinished covers fail |
| ACT weekend borrow and financed cash balances | Hand-calculated collateral, weekend and cash-interest tests |
| Bounded fills and price impact without double charging | Opening-event capacity/impact golden |
| Historical SEC/TAF inputs and separate fee reconciliation | Dated sale-levy golden; no current rate is hardcoded |
| Insolvency, closing maintenance, causality and determinism | Margin/insolvency cases, causal-prefix and Hypothesis conservation |
| Genuine computation, CLI, immutable replay and source corruption | `test_market_workflow.py` with real factor evaluation and installed subprocess |
| Late revisions, ambiguous sources, clocks and protected samples | Capture/terms/action negative cases and inherited v1 guards |

The initial kernel run passed 23 tests. The combined portfolio, factor-worker
and transform regression passed **120 tests in 326.58 seconds**. After tightening
source-scope rejection before raw-artifact access and fee-cap cent precision,
the final market suites passed **46 tests in 107.99 seconds**. Ruff lint/format
and strict mypy passed (49 research source files). These runs preceded the
repository-wide naming task; no financial formula changed during that rename.
The post-rename market suites pass **46 tests in 132.09 seconds**, including the
installed CLI and complete byte replay. Ruff lint/format and strict mypy pass
again. Publication and remote acceptance remain pending. Local verification
runs serially to avoid the prior task's native-IO contention.

Rollback: disable v2 writers or revert the task implementation. Keep input
captures, output ledgers and receipts; source changes invalidate current replay
without relabeling immutable historical results. No destructive migration exists.

Remaining after this unit: Phase 7 unit 3 statistics/multiple testing and unit 4
authorized execution/current reads/exports/admission; Phase 8 independent review.
