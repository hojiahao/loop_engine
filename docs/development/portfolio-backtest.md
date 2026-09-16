# Development portfolio replay

The installed Python worker supports two explicit development portfolio profiles.
ADR 0029 specifies the original fixed long-only ledger; ADR 0030 adds PIT actions,
long/short accounting, borrowing, financing and capacity. This administrative
command does not submit a runtime job or admit a factor. Paid data, protected
samples, production eligibility and independent verification remain gated.

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

## Version 2: actions, shorts and capacity

Use the same `backtest-run` / `backtest-validate` commands and v1 request envelope,
but freeze these explicit policy settings before the factor computation:

| Role | Exact required settings |
| --- | --- |
| `portfolio_policy` | `algorithm=ranked-long-short.1`, `holdings`, `initial_cash_usd`, `lot_size`, `long_weight_bps`, `short_weight_bps`, `initial_margin_bps`, `maintenance_margin_bps` |
| `execution_policy` | `algorithm=pit-next-open.1`, `participation_bps` |
| `cost_policy` | `algorithm=commission-impact-finance.1`, `commission_per_share_usd`, `minimum_commission_usd`, `half_spread_bps`, `impact_bps`, `short_collateral_bps`, `cash_debit_bps`, `cash_credit_bps`, `day_count` |

The other six roles keep the same identity and validation rules as v1. Integer
values must use canonical unsigned decimal text. Weights sum to 1..20000 bps,
and gross weight times initial margin cannot exceed 100,000,000. Initial margin
is 5000..10000 bps; maintenance is 2500..initial. Participation is 1..10000 bps;
impact at full event volume is 0..5000 bps; half spread is 0..1000. Short collateral
is 10000..30000 bps, cash debit rate 0..100000, credit rate 0..10000, and day count
is exactly 360 or 365. The original cash, holdings, lot and fee bounds still apply.
These are model parameters; no regulatory/broker margin rules are certified.

The tape has this shape, replacing the v1 CSV reference:

```json
{
  "schema": "loop.execution-tape/v2",
  "quality": "synthetic",
  "currency": "USD",
  "price_basis": "raw",
  "coverage": "explicit_development_declaration",
  "capture": {"sha256": "sha256:<actual digest>", "byte_size": 123}
}
```

The capture is a strict `loop.execution-capture/v1` object with `captured_at`,
`prices`, `terms`, `actions` and `sources`. All timestamps are RFC3339 UTC-aware
instants with exact millisecond precision; sessions are ISO dates. Every record
contains `security_id`, `session`, `effective_at`, `known_at`, `ingested_at` and
the existing `SourceEvidence` fields (`source`, `dataset`, `revision`, `record_id`,
`raw_sha256`, `availability`). `sources` contains the sorted unique bounded
`CachedObject` references for exactly those raw digests. Put their actual bytes
in the private evidence directory. The program verifies integrity and declared
PIT consistency; it does not certify normalization or vendor coverage.

| Record | Additional required fields and behavior |
| --- | --- |
| Opening price | `kind=open`, `price_usd`, `auction_volume` (0..1e12 shares). Effective time inside the session; first dissemination within 60 seconds; modeled fill at dissemination. No daily final volume substitution. |
| Closing price | `kind=close`, `price_usd`, `auction_volume=null`. Effective time identifies the scheduled close. Latest already visible revision supplies the mark. |
| Trading terms | `valid_until`, `tradable`, `short_allowed`, `borrow_limit` (absolute shares), `borrow_rate_bps`, `recalled`, `sec_fee_usd_per_million`, `taf_fee_usd_per_share`, `taf_fee_cap_usd`. No default zero rates or unlimited borrow. |
| Split | `kind=split`, `event_id`, positive integer `numerator` and `denominator` (each <=1e6). Optional paired `fraction_price_usd` and `pay_at` become mandatory when the actual position leaves a fractional residual. |
| Ordinary dividend | `kind=dividend`, `event_id`, positive `amount_per_share_usd`, `pay_at`. Creates a signed claim on pre-ex-date shares. |
| Final cash delisting | `kind=delisting`, `event_id`, nonnegative `amount_per_share_usd`, `pay_at`. Cancels orders, exchanges the position for a signed claim and permanently retires the ID. |

Prices use the v1 raw-USD bounds. Fees use at most eight decimal places. The SEC
pass-through rate is at most 1000 USD/million; TAF rate at most 1 USD/share and
cap at most 1e6 USD/order in whole cents. A positive TAF rate requires a positive cap. These
time-scoped declarations must reflect the intended historical assumptions, not
today's rates copied into every year. Values in tests are invented examples.

`effective_at <= known_at <= ingested_at <= captured_at` applies to price
observations. Announcements/terms may be known before becoming effective. A
first-observed source requires `known_at=ingested_at`; a later API download cannot
be relabeled as historically executable. Events and terms are selected using
public knowledge at each opening/closing event and the pinned ingestion cutoff.
Late revisions are retained in lineage but cannot rewrite prior fills.

Only one corporate event per security/session is supported, effective before
the scheduled opening and already known then. Supply actual ex-dates and payment
times. Complex/combined actions, special-dividend due bills and unresolved
consideration are unsupported; do not encode them as ordinary cash dividends.
An incomplete declared action list is not evidence of a complete historical feed.
SEC/Alpaca daily development captures do not by themselves supply these auction,
borrow and action inputs. No paid download or automatic source inference occurs.

Version 2 retains the seven artifact names but adds `reason` to orders,
`receivable_usd`, `gross_value_usd` and `short_collateral_usd` to NAV. Receivable
is the signed net of action assets/liabilities; the event ledger permits their
separate reconstruction. `costs` records `event_id`, `session`, `security_id`,
`kind`, `cash_delta_usd`, `receivable_delta_usd`, `commission_usd`, `sec_fee_usd`,
`taf_fee_usd`, `spread_cost_usd` and `impact_cost_usd`. Actions/claims/payments,
borrow/cash interest and actual trades all reconcile through these columns.
The receipt's engine is `pit-actions-long-short.1`; v1 readers must not interpret
these CSVs using the old cost/NAV schema.

Use ADR 0030 for exact selection, rounding, financing and failure rules. Notably,
unpaid positive claims are not spendable cash or initial-margin collateral,
short liabilities reserve cash, recalls require an actual cover, and missing
held marks/borrow terms invalidate the run. There is no synthetic liquidation
to avoid a margin/insolvency failure. The daily financing model uses prior-close
balances for the whole actual-day interval; it is not brokerage settlement.

Executable examples and hand-ledger acceptance:

```sh
./scripts/uv-research.sh run --locked --offline --no-sync pytest \
  tests/test_market_portfolio.py tests/test_market_workflow.py \
  tests/test_portfolio.py tests/test_backtest_workflow.py
```

The workflow fixture builds actual source objects, a v2 capture, frozen policies
and a real factor evaluation before invoking the installed CLI. It checks all
artifact bytes on replay and never treats imported performance metrics as a
computed backtest. Rollback disables v2 writes or reverts its task commit while
preserving immutable receipts and sources; v1 requires no migration.
