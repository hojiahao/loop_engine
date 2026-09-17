# ADR 0030: Point-in-time actions, financing and executable capacity

- Status: Accepted; `4953dc9` is pushed and CI `35082001411` passes all seven jobs
- Date: 2026-09-16
- Owner: hojiahao

## Requirement and boundary

Phase 7 unit 2 must account for splits, ordinary dividends, final cash delisting,
short availability, borrow costs, financing and liquidity in an actual replay.
Reusing an adjusted closing-price series as raw execution data would double count
actions and conceal missing availability. Administrative v1 replay remains valid
under ADR 0029; no existing policy silently acquires new financial semantics.

Use the same factor recomputation, nine frozen policy identities, private CAS,
seven ledger artifacts, receipt-last publication and read-only verification.
Add no dependency, service, database migration, production RPC or data purchase.
The new opt-in engine is `pit-actions-long-short.1`, requiring the v2 execution
tape and explicit new portfolio/execution/cost algorithms. Only synthetic and
public-development evidence in the existing IS/development ranges is accepted.
All receipts remain `production_eligible=false`.

## Evidence and causality

An immutable execution capture contains typed raw opening-auction prints,
closing marks, trading/borrow/fee terms and corporate events. Every record has
stable security identity, effective/public/ingestion times and a raw-source
SHA-256. Resolve the exact bounded source set and recheck it before publication.
Public time is the historical visibility cutoff; ingestion must be no later
than the pinned capture. First-observed sources cannot backdate their knowledge.
Raw bytes establish integrity and permit inspection of the normalization, not
vendor certification of the caller's declarations or complete action coverage.
This unit does not manufacture missing borrow/auction history from Alpaca daily
bars, Sharadar adjusted OHLCV or CRSP total returns.

Use the first disseminated opening print, within 60 seconds of its effective
event and inside the scheduled session. Model execution at dissemination time;
this remains an auction-price research approximation, not guaranteed access to
that auction. Later corrections cannot rewrite a fill. For closing marks, select
the latest revision already public at the factor decision. Preserve invisible
revisions in the input identity without using their values. Resolve terms at the
actual opening event and again at the close; an expired later effective state
cannot fall back to an older active state. Missing trading terms cancel new
exposure; a held short without current borrow terms invalidates the run.

Events must match the exact session/security grid; unknown or protected business
dates, ambiguous revisions and inconsistent clocks fail. Source references are
private administrative evidence, not capabilities. Authenticated registry and
runtime result publication remain unit 4.

## Corporate actions

- A split adjusts actual holdings, reference marks and previously fixed targets
  before opening execution. Use rational share conversion and truncate residuals
  toward zero. A fractional residual requires an already known cash-in-lieu
  price and payment time; record its signed claim rather than dropping value.
- An ordinary cash dividend accrues on the pre-ex-date position before opening
  trades. Longs receive an asset; shorts owe the lender. Payments change cash
  only at their declared payment time. Positive unpaid claims contribute to NAV
  but do not supply initial-margin collateral or earn cash interest. Negative
  claims reserve cash even when offset by positive claims in net NAV.
- A final cash-only delisting exchanges the position for its explicit signed
  payout, cancels pending orders and permanently retires the security. Zero is
  accepted only as an explicit payout, never inferred from absent quotes. Future
  selection or trading of that retired ID fails. No legacy delisting return is
  added to an already inclusive CRSP CIZ return.

One action per security/session is supported. Combined events, mergers/spinoffs,
stock dividends, due-bill/special-dividend arrangements, unresolved fractional
settlement or unknown delisting consideration need another reviewed profile and
are rejected. Do not infer an ex-date from a payment/record date. Claims beyond
the final sample remain marked claims; no terminal payment or liquidation is
invented. No withholding taxes or reorganization elections are modeled.

## Positions, execution and costs

Rank the same causal factor in its frozen direction. Allocate configured long
and short NAV weights independently, with disjoint top/bottom selections and
stable ID ties. With both legs configured, fewer than twice the required names
targets cash. Whole-lot targets use the preceding close. Splits can leave whole
odd lots, which can be closed in whole shares. Gross target weight is at most
200%; explicit initial/maintenance margins are model parameters, not claims of
regulatory or broker compliance.

At each event time, apply known actions/payments, update simultaneous observed
marks, then sell before buy with stable security-ID ties. A later opening price
cannot fund or value an earlier trade. Restrict fills to the frozen fraction of
that opening event's observed auction volume; never use the day's final volume.
The unfilled order expires. New short exposure additionally requires permitted
short sales and an absolute borrow limit. A recall or decreased opening limit
forces covering to the permitted position; an unavailable/halted/thin opening
that cannot complete it fails the replay. A withdrawal first known intraday
while a short remains held also fails; the daily model cannot invent an
intraday buy-in.

Impact is a disclosed linear research assumption:

`adverse bps = half_spread_bps + impact_bps * filled_shares / auction_volume`.

Recompute the modeled price for the final quantity. Bound initial margin by a
monotone whole-share search after the risk-reducing portion. Prices round
adversely to 1e-8 USD. Commission is the greater of per-share and minimum fees.
Explicit time-scoped SEC/TAF pass-through rates apply to sales: notional rate
per million and capped per-share rate, each rounded up to cents per modeled
order. Defaults are not silently populated with today's regulatory rates. This
is a declared retail pass-through model, not calculation of an SRO's aggregate
statutory obligation. Separate commission, SEC, TAF, spread and impact columns
explain costs; spread/impact already reside in fill prices and are not charged
again.

Cash is economic trade-date cash, not settled brokerage cash. Cash borrowing is
allowed only by the model's margin constraints. Between adjacent observed
sessions, accrue on prior-close balances for the actual calendar-day interval
using the frozen 360/365 denominator. Borrow fees use each held short's visible
prior-close rate and `shares * ceil(raw_mark * collateral_bps / 10000)` in USD.
Cash financing uses cash minus short collateral and unpaid dividend liabilities;
apply explicit debit/credit rates, without a double credit on short proceeds.
Positive credits round down and costs up to 1e-8 USD. This constant-interval
accrual does not simulate settlement dates, intraday rate changes or a broker's
full financing schedule. Payments between sessions become available at the
next observed event; prior-close accrual over that interval remains the frozen
approximation.

Mark holdings at visible raw closes. NAV is cash plus signed action claims plus
signed market value; returns remain successive NAV ratios. Insolvency at an
observed event or after costs and closing maintenance-margin breaches invalidate
the replay. There is no fictional fill to restore solvency. Keep these failures
distinct from factor rejection.

## Verification and recovery

Use hand-calculated action, fractional, short-liability, borrowing, weekend,
cash-interest, capacity/impact, dated-fee, missing-mark and insolvency goldens.
Require causal-prefix and flat-market conservation properties. Exercise the
installed CLI from genuine frozen factor computation, full artifact replay,
source corruption, unavailable/future revisions, unknown policies and holdout
date rejection. Retain v1 ledger and worker regressions. Budgets and interruption
publication semantics remain ADR 0029's; no new concurrent state writer exists.

Rollback disables the v2 profile or reverts this task's implementation. Retain
old captures, ledgers and receipts. Readers distinguish engine/tape versions;
the v1 numerical profile is unchanged. Source changes invalidate current replay
through existing provenance rather than rewriting earlier receipts. No database
down-migration or research-history deletion is required.

## Primary references

- [SEC short-sale, locate and dividend obligations](https://www.sec.gov/investor/pubs/regsho.htm)
  (the archival page's settlement-cycle example is not used here).
- [Investor.gov dividend entitlement and special cases](https://www.investor.gov/introduction-investing/investing-basics/glossary/ex-dividend-dates-when-are-you-entitled-stock-and)
- [Investor.gov reverse splits and fractional cash-outs](https://www.investor.gov/introduction-investing/investing-basics/glossary/reverse-stock-splits)
- [IBKR short collateral convention](https://www.interactivebrokers.com/campus/glossary-terms/collateral-short-sale/)
- [SEC Section 31 basis and changing rates](https://www.sec.gov/rules-regulations/fee-rate-advisories/section-31-transaction-fees-basic-information-firms)
- [FINRA historical per-share/capped TAF rule example](https://www.finra.org/sites/default/files/RuleFiling/p179403.pdf)

These sources motivate explicit inputs and distinctions. They do not certify
this simulator, its declared inputs or its synthetic policy values.
