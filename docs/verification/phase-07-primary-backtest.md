# Phase 7 primary portfolio backtest verification

## Unit 1: next-session ledger (`implemented`; publication gate pending)

Requirement and scope: ADR 0029. Implementation provides bounded administrative
`backtest-run` and read-only `backtest-validate`, using actual recomputed factor
evidence, all frozen policy identities and explicit raw execution observations.
No service, database table, dependency or production-data permission is added.

| Requirement | Executable evidence |
| --- | --- |
| Cash/holdings/NAV reconciliation with opening gaps | `test_hand_accounting_with_opening_gaps` |
| Successive NAV returns, absent first return | `test_returns_are_ratios_and_first_is_missing` |
| Fixed-share next-session orders, causal prefixes | `test_next_open_never_resizes_the_decision_order`, `test_future_signals_cannot_change_prior_ledger` |
| Chronological sale funding | `test_late_sale_cannot_fund_an_earlier_open` |
| Commission/spread without double charging | `test_commission_and_spread_goldens` |
| Missing observations and explicit terminal holdings | `test_unfilled_open_expires_and_cannot_use_the_close`, missing-mark tests, `test_final_positions_are_marked_without_hidden_liquidation` |
| Deterministic ties, direction, decimal context and conservation | Direction/tie/context tests and `test_flat_frictionless_market_conserves_wealth` |
| Actual numerical source and installed CLI | `test_real_evaluation_to_ledger_and_read_only_replay`, `test_installed_cli_runs_and_replays` |
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

Remaining Phase 7 units: action/financing/borrow/capacity accounting and PIT
execution inputs; statistics/multiple-testing; authorized runtime completion,
current reads/exports and admission. Independent validation remains Phase 8.
Licensed historical production-data coverage remains deferred by the owner.

Rollback: disable new portfolio writers or revert the unit's code commit,
preserving immutable inputs/results/receipts. No schema migration or deletion of
research/audit history is needed. Temporary test directories may be removed
after evidence is recorded; actual market captures and research artifacts remain.
