# Phase 8 independent validation evidence

Phase 8 is in progress. The accepted Phase 7 baseline is `549cb11`; its exact
GitHub Actions run `35202271352` passed all seven jobs. Phase 5 licensed historical
coverage remains deferred; synthetic/development verification cannot close it.

## Unit 1: Alphalens statistics

Requirement and design: [ADR 0033](../adr/0033-independent-statistical-validation.md).
Usage, interpretation and rollback:
[independent statistics](../development/independent-statistics.md).

The change adds an actual administrative export/reconciliation workflow, not
only a dependency installation. Primary export first recomputes the frozen
factor, portfolio and statistical evidence. The separate process independently
forms labels/groups and invokes Alphalens for Rank IC, group means and quantile
membership turnover. SciPy computes Pearson IC and group monotonicity. It does
not import the primary numerical package or consume its ranks/return kernel.
Raw observations and primary export normalization remain shared dependencies.

Compatibility was exercised with CPython 3.14.4, Alphalens Reloaded 0.4.6,
pandas 2.3.3, NumPy 2.5.2 and SciPy 1.18.1. The primary environment retains
pandas 3.0.5 and the only persistent project `.venv`. The independent lock,
bootstrap, package checks, tests and build are included in existing CI jobs.

Acceptance cases include:

- Hand-calculated Pearson/Rank IC, group means, spread and monotonicity; tied
  signals, reverse direction and unequal group sizes.
- Constant signal/labels, insufficient coverage, missing prices, terminal
  unlabeled date, weekend/holiday alignment and unavailable-date turnover.
- Real installed primary-export and isolated Alphalens subprocesses over a
  17-session, six-security frozen factor/portfolio fixture: 16 available
  comparisons, zero differences and byte-identical read-only replay.
- Deliberately incorrect primary Rank IC reported per date; insufficient sample
  explicitly unavailable; malformed CSV/JSON, altered bytes, changed validator,
  symlink/replaced store and input corruption rejected.
- Interruption during publication leaves no final receipt and preserves original
  inputs; elapsed deadlines and clock regression reject execution.

Local numerical acceptance: 37 independent-validator tests passed in 129.40s;
23 affected primary-export/statistics regressions passed in 504.89s. The latter
includes actual CLI export, isolated calculation and read-only replay. Ruff and
strict mypy passed for both packages; the validator's source and wheel build
passed, as did the root lock/environment and function-naming checks.

Final full `just check` passed, including Rust formatting/Clippy with warnings
denied, cross-language protocol compatibility, TypeScript checks, both Python
packages and naming. The initial restricted run hit Node `spawnSync git` EPERM;
the ordinary host rerun retained all checks. Strict mypy then identified a missing
CSV row-list annotation, which was fixed before the successful final gate.
Exact-commit remote CI, commit and push are still required before this unit can
move from `in_progress` to `complete`.

## Remaining work and recovery

Unit 2 must independently execute and reconcile portfolio accounting with
Zipline. Unit 3 must bind both validator receipts to authenticated, registered
primary evidence and shared admission. Passing this statistical comparison
does not permit formal factor admission; licensed-data, semantic-review and
multiple-testing requirements also remain enforceable.

Rollback disables new administrative writers or reverts the task commit while
preserving raw inputs, primary/independent receipts, differences and audit
history. No database migration, production connection, paid download, holdout
unlock or admitted-factor mutation is part of unit 1.
