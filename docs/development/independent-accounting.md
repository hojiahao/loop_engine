# Independent portfolio accounting

The Phase 8 Zipline worker independently calculates both frozen portfolio
profiles from raw execution observations. It uses actual Zipline order/position
accounting, without importing `loop_research` or consuming primary orders as
execution instructions. [ADR 0034](../adr/0034-independent-accounting-validation.md)
specifies extension points, limitations, precision and rollback.

This administrative workflow accepts only synthetic/public development data
within the existing 2007–2020 search/development ranges. It has no database,
discovery or holdout capability. A passing report is not a production factor
admission or proof of licensed survivorship/PIT coverage. Authenticated report
registration and the shared admission gate remain the next delivery unit.

## Install and run

`./scripts/bootstrap.sh` installs the separately pinned validator. Normal
invocations use an ephemeral uv environment, not a second project `.venv`:

```bash
./scripts/uv-zipline.sh run --locked --offline loop-zipline doctor
```

The main research service stays on Python 3.14.4/pandas 3.0.5. This validator
uses Python 3.12.13/Zipline Reloaded 3.1.1/pandas 2.3.3. Upstream bcolz needs
setuptools 80.9.0; its deprecation warnings remain visible. An upgrade requires
new compatibility and numerical evidence.
During bootstrap, the pinned secondary interpreter may be downloaded even when
the development image disables implicit Python downloads. The exception is
scoped to that one bootstrap command; subsequent checks and runs remain offline.

First obtain an actual portfolio receipt using the
[portfolio workflow](portfolio-backtest.md). Use its existing private evidence,
read-only view and output store, substituting absolute paths and the receipt
SHA-256 below. Do not place market data in the Git checkout.

```bash
./scripts/uv-research.sh run --locked --offline --no-sync loop-research \
  zipline-prepare --backtest sha256:<primary-receipt> \
  --evidence /absolute/private/evidence \
  --view /absolute/readonly/view \
  --store /absolute/private/results

./scripts/uv-zipline.sh run --locked --offline loop-zipline run \
  --input sha256:<exported-input> --store /absolute/private/results

./scripts/uv-zipline.sh run --locked --offline loop-zipline validate \
  --receipt sha256:<independent-receipt> --store /absolute/private/results
```

Export recomputes the factor and portfolio first. The immutable input binds raw
signals/eligibility, opening/closing clocks/prices, resolved borrow/fee terms,
events, the frozen policy and primary comparison artifacts. Source data and PIT
selection remain a common dependency; the independent engine does not assert
that caller-declared public data has production-quality historical coverage.

Only a canonical, current-user-owned, mode-0700 private CAS is accepted. All
objects are checksum-verified and bounded. The worker writes artifacts before
its final receipt; replay writes nothing and never repairs corrupt evidence.
Both paths recheck input/build bytes and enforce a 180-second cooperative budget.
Supervisors should also impose a hard subprocess deadline and cancellation.

## Interpreting results

Exit status is 0 for an accepted diagnostic, 3 for accounting differences,
4 for unavailable numerical precision, 2 for invalid evidence/policy/build or
budget failure, and 130 for cancellation. Infrastructure/invalid evidence is
never recorded as a rejected economic factor by this command.

The JSON output references:

- seven independently computed ledgers: targets, orders, fills, positions,
  cash/NAV, simple returns and costs;
- a bridge showing native Zipline NAV, signed unpaid entitlements and economic
  NAV for every session;
- per-field differences with fixed tolerances, plus total/omitted counts;
- actual validator/interpreter/dependency byte identities and a summary.

The economic bridge is mandatory. Native Zipline does not include unpaid
dividend claims in NAV and handles fractional splits differently. The adapter
records explicit delayed claims, signed short obligations, whole-share changes,
cash delisting and internal financing flows. A future payment is never pulled
forward to make a comparison pass. Native and economic NAV are both retained.

IDs, order statuses, quantities, session ordering and missingness compare exactly.
Absolute tolerances are 5e-9 USD for prices, 1e-5 USD for monetary totals and
1e-12 for returns, with no relative tolerance. Float precision outside this
reviewed range is unavailable. Do not alter tolerances after seeing a mismatch;
investigate inputs, execution assumptions and the recorded difference instead.

Unsupported events/profiles, missing held marks, unresolved fractional payments,
unmet recalls and invalid margin states fail closed. Default Zipline daily-bar
trading is not used: explicit observed opening events drive the finance components.

## Verification and recovery

```bash
./scripts/uv-zipline.sh run --locked --offline pytest
./scripts/uv-research.sh run --locked --offline --no-sync pytest tests/test_zipline_inputs.py
```

Tests cover handwritten accounting goldens and actual installed subprocess
reconciliation of both primary profiles. Full workspace/CI commands include the
isolated package. [Acceptance evidence](../verification/phase-08-independent-validation.md)
tracks commit/push status separately from implementation.

Rollback disables these new administrative writers or reverts the task commit.
Keep immutable source inputs, differences and receipts. No database migration,
holdout reopening, audit rewrite or destructive repair is required. Disposable
vendor cache directories are removed on process exit; ordinary test scratch
directories can be deleted after their results are recorded.
