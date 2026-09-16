# Development portfolio replay

Phase 7 unit 1 adds an actual portfolio ledger using installed Python numerical
code. ADR 0029 specifies its fixed long-only model. This administrative command
does not submit a runtime job or admit a factor. Paid data, protected samples,
shorting, corporate-action accounting and independent verification remain gated.

## Commands

After the normal workspace bootstrap, use the existing Python environment:

```sh
./scripts/uv-research.sh run --locked --offline --no-sync loop-research backtest-run \
  /absolute/private/request.json \
  --evidence /absolute/private/evidence \
  --view /absolute/readonly/factor-view \
  --store /absolute/private/portfolio

./scripts/uv-research.sh run --locked --offline --no-sync loop-research backtest-validate \
  --receipt sha256:REPLACE_WITH_RETURNED_RECEIPT_DIGEST \
  --evidence /absolute/private/evidence \
  --view /absolute/readonly/factor-view \
  --store /absolute/private/portfolio
```

These paths/digests are placeholders for administrator-prepared evidence. The
evidence/output directories must already exist with owner-only mode 0700.
The factor view uses the existing immutable read-only leaf contract (directory
0555, files 0444) and must be separate from both private caches. All input files
use their SHA-256 hex as filenames; references include exact byte counts. No
remote URL, arbitrary executable or credential is accepted.

The command returns receipt and seven artifact references, quality, session/
order/fill counts and ending NAV. `production_eligible` is always false. Replay
does not write files. A current replay fails if any input, result, installed
source, native environment or frozen policy differs. Old records are retained.

## Preparing the frozen inputs

The request schema is `loop.portfolio-request/v1`, with these required fields:

| Field | Required actual bytes |
| --- | --- |
| `evaluation_work` | Serialized existing `FactorEvaluationWork` used for the factor computation |
| `evaluation_result` | Its canonical `loop.factor-evaluation-result/v1` or `/v2` JSON manifest |
| `factor_values` | The corresponding complete factor CSV |
| `execution_tape` | The separate execution-tape manifest described below |
| `policies` | Actual documents keyed by all nine `*_policy` roles from the FactorSpec |

Each reference is `{"sha256":"sha256:<64 lowercase hex>","byte_size":123}`.
The original factor view must retain the exact panel and any exposure objects.
The command recomputes values from the canonical AST and checks the result
manifest, including factor, direction binding, sample, seed, quality, transforms
and coverage. Merely editing an input CSV and its hash cannot pass this check.
Source and environment must match the actual installed implementation.

Choose the following settings **before** freezing the FactorSpec and evaluating
the factor. Every actual document must match its recorded ID/revision/SHA-256;
adding a portfolio policy afterwards requires a new FactorSpec/evaluation.
Policy document format is the existing `loop.research-policy/v1`, with sorted
string-valued `settings` and the canonical field order used by the materializer.

| Policy role | Exact supported settings in this profile |
| --- | --- |
| `portfolio_policy` | `algorithm=long-only-top-n.1`, `holdings=1..1000`, positive `initial_cash_usd`, `lot_size=1..10000` |
| `execution_policy` | `algorithm=next-session-open.1` |
| `cost_policy` | `algorithm=commission-spread.1`, nonnegative `commission_per_share_usd`, `minimum_commission_usd`, integer `half_spread_bps=0..1000` |
| `evaluation_policy` | Integer `minimum_coverage_bps=0..10000`; zero eligible cells still fail |
| `universe_policy`, `data_policy`, `calendar_policy` | Empty settings; use the already built panel's explicit selection and calendar |
| `preprocess_policy`, `neutralization_policy` | Empty for raw v1; exact implemented transformation documents from the v2 panel otherwise |

The administrator remains responsible for selecting the correct runtime input
records. This command verifies bytes and computation, but job/lease IDs and the
recorded configuration/data registry hashes do not authenticate the caller or
certify a durable trial. The authorized runtime integration is Phase 7 unit 4.

## Execution tape

The tape has the following shape, with an actual reference in `observations`:

```json
{
  "schema": "loop.execution-tape/v1",
  "quality": "synthetic",
  "currency": "USD",
  "price_basis": "raw",
  "corporate_actions": "none_in_sample_declared",
  "observations": {"sha256": "sha256:<actual digest>", "byte_size": 123}
}
```

Quality must equal the factor panel's `synthetic` or `public_development` grade.
The CSV covers every evaluation session/security cell, ordered exactly as the
factor panel, excluding warmup:

```csv
session,security_id,open_at_ms,open_usd,close_known_at_ms,close_usd
2010-01-04,US.001,1262615400000,10,1262638800000,10
```

The row above is invented format evidence. Timestamps are UTC milliseconds;
the calendar checks scheduled opening/closing times and half-days. Actual
opening timestamps can be later than the scheduled session open. Both price
and timestamp must be empty for a missing observation. Missing openings cancel
that session's order. Closing prices must already be visible at the factor
decision. A missing held or selected closing mark fails the run.

Raw prices are positive, between 0.000001 and 1,000,000,000 USD, with at most
eight decimal places. The initial capital is positive and at most 1e15 USD;
orders cannot exceed 1e12 shares. No adjustment, split or dividend is silently
applied. A supplier daily OHLC bar does not itself establish an executable open;
this first tape is an explicit development declaration. Source-backed PIT tape
construction and action/availability evidence belong to unit 2.

## Ledger semantics

- At the close, select eligible finite scores in the frozen direction; ties
  use the stable security ID. Fewer than the requested count are equally
  weighted across the available selected names. No valid scores targets cash.
- Use closing NAV/prices for whole-lot target shares. Trade their differences
  at the next declared opening events. Never use tomorrow's price to size the
  decision. Event order controls funding; equal-time sales precede purchases.
- Model adverse half-spread and per-order commission. Funding can reduce buys
  to whole lots. Each order records requested/filled shares and `filled`,
  `partial_cash`, `cash` or `missing_open`; its unfilled balance expires.
- Cash is economic trade-date cash, immediately reusable after a modeled sale;
  it is not an assertion about settled cash, broker permissions or financing.
- Mark all holdings at visible raw closes. NAV equals cash plus marked value.
  Returns use successive NAV ratios, with an empty first observation; never
  delta-NAV. The last session retains its marked positions without liquidation.

`targets`, `orders`, `fills`, `positions`, `nav`, `returns` and `costs` are
separate immutable CSV artifacts. Modeled execution prices round adversely to
1e-8 USD; other cash entries remain exact at that precision. Returns round to
18 decimal places. Spread is already in the fill price; the cost ledger reports
it for explanation and must not subtract it a second time.

## Reproducible acceptance and recovery

Run the real worker/CLI integration and independent hand-ledger cases:

```sh
./scripts/uv-research.sh run --locked --offline --no-sync pytest \
  tests/test_portfolio.py tests/test_backtest_workflow.py tests/test_factor_worker.py
```

The installed CLI case creates synthetic immutable inputs, computes the actual
factor, executes the command in an isolated Python subprocess and replays every
output byte. It never substitutes imported performance numbers for computation.
Goldens cover price gaps, cash constraints, fees/spread, opening chronology,
fixed direction, missing observations, causal prefixes and terminal positions.

Ctrl-C exits 130. IO, corruption, unknown policies and budgets fail without a
successful report. Publication uses unique temporary files and no-overwrite CAS;
interrupted publication may leave standalone immutable objects. Only the final
receipt establishes completion. Rerun the same request to reuse identical bytes;
corruption is rejected, not silently repaired. Preserve original inputs and
historical receipts during rollback. Disable new `backtest-run` use or revert
this task commit; no database migration is required.
