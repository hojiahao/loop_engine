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
Correction `7258607` is pushed. Exact-commit CI
[35319727700](https://github.com/hojiahao/loop_engine/actions/runs/35319727700)
passed all seven jobs. The first container attempt stopped at a DaoCloud
registry HTTP 503/TLS timeout, before project bootstrap. Retrying only that
failed job, with the same commit, DaoCloud sources and image digests, passed
the complete clean-container gate in 27m38s. The unified workspace gate passed
in 35m18s and the standalone Rust gate in 23m55s. Unit 2 is accepted.

Only the failed probe's source builds, unused Python 3.13.14 interpreter and
identified CPython-3.13 numerical caches were removed (roughly 0.6 GiB). They
can be downloaded again. No data snapshots, audit history, unrelated project
files or active Python 3.12/3.14 environments were removed.

## Unit 3: Authorized reconciliation (acceptance in progress)

Requirement and recovery: [ADR 0035](../adr/0035-authorized-reconciliation.md).
Deployment and RPC usage:
[authorized reconciliation](../development/authorized-reconciliation.md).

The optional mTLS path binds one registered primary job to both installed
validators and a deployment-pinned comparison policy. It reconstructs the actual
primary and global trial evidence, supervises both independent processes, then
registers the immutable report through the existing lease-fenced PostgreSQL
job/receipt/audit transaction. No new table, migration or numerical dependency is
introduced. Generic completion cannot import an administrative success as proof.
Current reads and command retries are read-only numerical replays. The shared
admission/readmission handler retains all production prerequisites.

All 33 affected Python export/statistics regressions pass together, including the
six new authorized-export cases and actual isolated Alphalens/Zipline replay.
The export now shares one live verified primary reconstruction between its two
exporters, retaining the original final integrity guards; it does not cache
authority or reuse evidence across operations. Ruff formatting/lint and strict
mypy pass for the primary research package. The five Rust process-supervisor
tests pass environment isolation, cancellation of a descendant, output-bound
enforcement, worker-failure classification and rejection of empty success output.

Actual mTLS/PostgreSQL execution has reached accepted Alphalens/Zipline numerical
agreement, durable registration, restart command replay and current reads. Its
first admission check exposed the inherited 30-second envelope expiring during
independent replay. ADR 0035 now assigns the evidence-bearing path an explicit
180-second maximum; ordinary admission remains at 30 seconds. The completed
integration run verifies both this path and rejection above the bound.

Test-fixture corrections separate ma(2) warmup from the statistical/execution
sample and use the declared factor-panel-value schema. Production validators
were not weakened. Protocol changes retain the historical pair-of-backtests
shape and add five shared input cases, extending the matrix to 114. TypeScript
contracts (115 tests), Python job contracts (138 tests), and TypeScript
format/lint/type gates have passed. The complete Rust suite, the corrected
8-process case, commit, push and exact-commit CI remain acceptance gates.

Only project Rust incremental compilation files were removed to recover about
1.8 GiB; they are reproducible. Sources, research artifacts, audit history,
installed dependencies and other projects' temporary files were preserved.

Interrupted interactive sessions did not produce a final Rust result. Local
acceptance therefore runs as a bounded transient task. Its first launch inherited
systemd's 1,024-descriptor soft limit and failed while pinning numerical manifest
files; the ordinary host uses 65,535. After matching that limit, the next launch
exposed a full 804 MiB `/tmp` filesystem. Six confirmed, inactive project test
directories were removed, releasing about 756 MiB and reducing `/tmp` usage from
100% to 6%. Those environmental failures do not count as passing acceptance.

The completed resumed integration run reports 13 passed, two failed and one
subprocess-only helper ignored in 4,217.81 seconds. Accepted registration,
restart/read replay, admission/override denial, unavailable evidence and both
commit-boundary crash recoveries pass. The 2/4-process commit cases pass before
the 8-process case reaches a parent marker timeout: that parent allowed 180s,
although each child first replays the primary (up to 180s) and then reconstructs
independent evidence (up to another 180s). The test setup budget is now 420s,
with 30s for the transaction where needed; all production bounds are unchanged.
The 2/4/8 cases now have separate test names. The second failure occurs during
the unregistered-source fixture's existing file-verification budget, before the
behavior under test. Its isolated rerun passes in 71.13s without changing that
production verification budget. Early decision-attribution denial passes in
68.27s, and all five subprocess-supervision cases pass in 0.34s, including the
new failure/empty-output distinction.

The corrected 8-process attempt stops during its factor-evaluation fixture,
before any competing writer starts: the real factor worker returns an
unavailable error. This is not a passed concurrency test. The host has about
1.6 GiB RAM; the bounded targeted run records about 936 MiB peak memory and
696 MiB peak swap use. These observations do not establish the failure's cause.
The full matrix must pass on the exact implementation commit in CI before
unit 3 or Phase 8 can be accepted; the failure is not skipped or suppressed.

Final local `just check`, `just build` and `just doctor` pass at
2026-09-20T05:39:03Z. Checks take 381.07s and build takes 191.46s. They include
Rust formatting and Clippy with warnings denied, protocol generation/wire/
compatibility checks, TypeScript and all four Python package style/type checks,
3,820 Python/Rust/shell and 343 TypeScript/JavaScript function-name checks,
all workspace/independent-package builds and the single Python 3.14.4 root
environment check. The disposable PostgreSQL test container has been stopped.
After recording these results, the completed runners' temporary scripts, logs
and owned scratch directory are removed; no research or audit artifacts are
part of that cleanup. The completed build's reproducible Rust incremental
cache is also removed, recovering about 1.2 GiB and leaving about 2.2 GiB free
on the root volume. Installed dependencies and the built executables remain.

Implementation `eaa638a` is pushed. Its CI run
[35493313870](https://github.com/hojiahao/loop_engine/actions/runs/35493313870)
passes six of seven jobs, including the full Rust and unified workspace gates.
The Rust log explicitly confirms all 2/4/8-process validation races and both
commit-boundary crash recoveries; the earlier local 8-process failure therefore
has actual passing CI evidence. Rust completes in 47m40s and the unified
workspace in 56m29s. The research job passes all 934 primary tests, 37 Alphalens
tests and 46 Zipline tests. The clean-container
job exposes two test-fixture path assumptions: it installs the secondary
interpreter under `LOOP_ENGINE_RUNTIME_ROOT`, while the new fixture looked only
under the source tree; the process-supervisor tests also assumed
`/usr/bin/python3`, whereas the image provides the pinned interpreter through
the workspace `.venv`. The correction follows bootstrap's runtime-root rule for
both the secondary interpreter and uv cache, and uses the primary workspace
interpreter for all five supervisor cases. All five pass locally in 0.40s;
formatting, Clippy with warnings denied (9m12s), and the 3,821-declaration naming
check pass. No production numerical,
authorization, comparison or deadline rule changes. The correction's container
and full exact-commit CI remain required before phase closure.

## Remaining work and recovery

Unit 3 must complete and publish the remaining acceptance evidence. Passing this statistical comparison
does not permit formal factor admission; licensed-data, semantic-review and
multiple-testing requirements also remain enforceable.

Rollback disables new administrative writers or reverts the task commit while
preserving raw inputs, primary/independent receipts, differences and audit
history. No database migration, production connection, paid download, holdout
unlock or admitted-factor mutation is part of unit 1.
