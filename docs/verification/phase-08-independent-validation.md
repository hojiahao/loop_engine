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
Commit `3be45a1` is pushed. Exact-commit CI
[35306864434](https://github.com/hojiahao/loop_engine/actions/runs/35306864434)
passed all seven jobs, including complete Rust/process regressions, Python
research with the new isolated validator, unified workspace commands and the
clean DaoCloud container. Unit 1 is accepted; Phase 8 remains in progress.

## Unit 2: Zipline accounting

Requirement, numerical bridges and rollback:
[ADR 0034](../adr/0034-independent-accounting-validation.md).
Executable commands: [independent accounting](../development/independent-accounting.md).

Both existing primary profiles have independent next-opening replay using
Zipline's actual blotter, transactions, commission application, position tracker
and ledger. Independent exact-rational decisions handle sizing, capacity,
dated costs, borrow, margin and corporate obligations. Primary ledger outputs
are comparison targets only. Raw inputs and PIT export normalization remain
shared dependencies. Native Zipline and economic NAV are retained side by side;
delayed claims and split/financing extensions are explicit, not hidden offsets.

Python 3.14.4 source installation reached its six-minute budget; the 3.13.14
bcolz source installation reached three minutes. Neither timeout proves those
versions cannot compile. Actual wheel-based imports and the doctor passed on
Python 3.12.13, Zipline 3.1.1, bcolz 1.2.10, NumPy 2.5.2, pandas 2.3.3 and
SciPy 1.18.1. bcolz requires the explicit setuptools 80.9.0 pin. The primary
environment remains Python 3.14.4/pandas 3.0.5 and the sole persistent `.venv`.

Handwritten goldens cover causal order sizing and earlier/later openings,
whole-share long/short fractional splits, delayed dividend and delisting claims,
SEC/TAF rounding, liquidity/impact, initial margin, borrow recall and weekend
financing. Negative tests cover unsupported actions, calendar gaps, future marks,
missing marks/borrow, policy drift, wrong quantities/prices/cash/returns,
precision denial, changed bytes/build, cancellation and clock regression.

The installed primary CLI and isolated Zipline worker reconcile both primary
profiles and reproduce all artifacts without writes on replay. All 57 affected
primary regressions passed in 658.47s, including portfolio, market-action,
Alphalens export and Zipline export tests. Primary/independent Ruff and strict
mypy pass, as do Rust formatting, Clippy with warnings denied, root lock and
single-environment verification and the 3,733-declaration naming gate.

A parallel local run exposed uv hardlink ctime races during dependency hashing:
42 independent tests passed and two stopped before calculation. Source reads
now retry at most twice with unchanged strict metadata checks and final full
byte revalidation. A deterministic hardlink-interleaving regression confirms
that dependency reads can recover while CAS evidence reads still reject the
same mutation. The final independent suite passed all 46 tests in 57.05s; the
source/wheel build also passed. Upstream deprecation warnings remain visible.
Implementation `ce4e246` is pushed. Its independent Zipline CI step passed, but
run `35317618068` exposed a clean-container bootstrap omission: the base image
sets `UV_PYTHON_DOWNLOADS=never` and has no secondary Python 3.12.13 interpreter.
Bootstrap now permits downloading only through the explicitly pinned validator
command; ordinary container runs retain the original no-download setting.
Cold-install verification used an empty, project-owned temporary interpreter
directory: `never` reproduced the failure; the bootstrap-scoped automatic
download installed the exact pin and passed the doctor; restoring `never` with
`--offline` also passed. The root Python 3.14.4 single-environment check passed.
The corrected exact-commit CI remains the final publication gate.

Only the failed probe's source builds, unused Python 3.13.14 interpreter and
identified CPython-3.13 numerical caches were removed (roughly 0.6 GiB). They
can be downloaded again. No data snapshots, audit history, unrelated project
files or active Python 3.12/3.14 environments were removed.

## Remaining work and recovery

Unit 2 awaits its final acceptance and publication. Unit 3 must bind both validator receipts to authenticated, registered
primary evidence and shared admission. Passing this statistical comparison
does not permit formal factor admission; licensed-data, semantic-review and
multiple-testing requirements also remain enforceable.

Rollback disables new administrative writers or reverts the task commit while
preserving raw inputs, primary/independent receipts, differences and audit
history. No database migration, production connection, paid download, holdout
unlock or admitted-factor mutation is part of unit 1.
