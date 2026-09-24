# ADR 0029: Frozen next-session portfolio accounting

- Status: Accepted; `cf2750d` is pushed and CI `35065117368` passes all seven jobs
- Date: 2026-09-16
- Owner: hojiahao

## Requirement

Phase 7 unit 1 must turn genuine computed factor values into inspectable target
positions, orders, fills, costs, cash, marked positions, NAV and simple returns.
A new arithmetic helper or imported performance JSON is insufficient. Preserve
the frozen factor direction, policies and actual input lineage, with an
executable administrative workflow before integrating the runtime in unit 4.

## Decision

Reuse the installed Python worker, canonical FactorSpec, read-only factor view
and immutable CAS publisher. Extract its existing computation/encoding path so
ordinary factor execution and portfolio replay use identical numerical checks.
Recompute the actual frozen factor and compare every values/manifest byte;
verify source/environment and live input guards before completing publication.
Resolve all nine policy documents against their FactorSpec identities. This
administrative path does not resolve a caller's job ID into runtime authority,
certify the caller's configuration/data registry, or bypass durable admission.

The first profile is `long-only-next-open.1`, with USD cash and whole lots.
At each visible close, rank eligible finite signals using the frozen direction
and stable security-ID ties. Select at most the frozen holdings count. Size
equal-value positions using that close's NAV and raw closing prices, rounding
shares down to the frozen lot size. Form fixed-share differences from actual
holdings. Tomorrow's opening price never participates in this decision.

At the next session's declared opening observations, process chronological
events, then sells before buys at equal timestamps, then stable security IDs.
Model price as raw open plus/minus the frozen half-spread; round adversely to
1e-8 USD. Commission is the greater of the frozen minimum and per-share fee.
Buys may be reduced to affordable whole lots, with the remainder explicitly
cancelled. Unobserved opens cancel that session's order; no close-price fill or
automatic carry-forward is invented. A later sale cannot finance an earlier
buy. Economic cash is available immediately at modeled execution; this is not
a settled-cash brokerage account or an implementation of settlement regulation.

Use a local 80-digit Decimal context independent of process-global settings.
USD inputs have at most eight decimal places, with explicit price, cash and
quantity bounds. Prices and fees produce exact cash movements at that precision.
Mark holdings at their visible raw close; a missing held/selected mark fails.
NAV equals cash plus marked holdings. Compute `NAV[t] / NAV[t-1] - 1`, reporting
18 decimal places. The first return is absent because no prior NAV is observed.
Start with cash and no positions; keep terminal positions marked without an
undisclosed liquidation or a trade outside the frozen sample.

The execution tape is a separate, declared raw USD product with a complete
session/security grid, per-security opening timestamps and close visibility.
Use the locked XNYS calendar for complete sessions, DST and scheduled half-days.
An opening observation may follow the scheduled open; it must precede the
scheduled close. Reject future-visible closing marks. This model does not claim
that every security opens at 09:30, or that daily bars guarantee executable
liquidity. NYSE documents delayed security openings; Investor.gov explains that
market-order execution prices are not guaranteed (references below).

No database, RPC, service or dependency is added. Publish seven bounded CSVs
using the existing no-overwrite publisher, then the final immutable receipt.
The request pins exact work/result/value/tape references and policy bytes. A
read-only replay rebuilds and compares all outputs under the current build;
source/environment drift invalidates current replay without rewriting history.
An interruption may leave CAS objects, never a fabricated successful receipt.

## Scope and limits

Only one IS (2007-2016) or development (2017-2020) window is allowed. The path
has no holdout capability and accepts only `synthetic`/`public_development`
inputs. `production_eligible` is always false. The no-corporate-action declaration
is an explicit input assumption, not independently certified historical coverage.
Unknown policy algorithms, shorting, adjusted execution prices and action modes
fail closed. A result is an administrative accounting receipt, not a registered
`BacktestResult`, admission decision, profitability claim or independent review.

The first profile caps replay at 100,000 cells, 64 MiB input/aggregate ledger
bytes and a 180-second cooperative deadline checked at stages, sessions and
fills. The inherited factor computation also retains its cell/work/build bounds.
No throughput SLA or hard interruption inside a native numerical call is claimed.
Corporate actions, financing/settlement, borrow, capacity/impact and PIT execution
source preparation belong to unit 2; statistics to unit 3; authenticated durable
completion/current reads/export/admission to unit 4. Phase 8 remains independent.

## Acceptance and rollback

Require hand-computed gap, cash, fee, chronology, marking and return goldens;
causal-prefix, direction, tie, missingness and Decimal-context properties; actual
installed CLI execution from real factor computation; byte-identical replay;
policy/provenance/coverage/clock/grid/corruption/deadline failures. Retain the
existing factor-worker and workspace gates because its computation is shared.

Rollback disables `backtest-run` or reverts the task commit; retain all input
artifacts, published ledgers, receipts and prior audit history. No schema or
destructive down-migration is involved. Historical receipts remain evidence,
but must not be relabeled as current when their recorded source differs.

## References

- [NYSE opening process](https://www.nyse.com/publicdocs/nyse/NYSE_Auctions_Opening_Process_Fact_Sheet.pdf)
- [Investor.gov order types](https://www.investor.gov/introduction-investing/general-resources/news-alerts/alerts-bulletins/investor-bulletins-14)
