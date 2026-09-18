# ADR 0034: Independent Zipline execution and accounting reconciliation

- Status: Implemented; task acceptance, commit and push pending
- Owner: hojiahao
- Extends: ADR 0005, ADR 0029, ADR 0030 and ADR 0033

## Requirement and compatibility evidence

Phase 8 unit 2 requires independent orders, fills, costs, positions, NAV and
returns from the actual frozen execution inputs. Feeding primary orders or
primary NAV into another library would not verify execution or accounting.
This remains a bounded administrative diagnostic. Authenticated reconciliation
and shared admission are unit 3; no production endpoint is enabled here.

Keep the primary CPython 3.14.4 workspace and its single persistent `.venv`.
Zipline Reloaded 3.1.1 has no usable CPython 3.14 wheel in the selected index;
a complete source install did not finish within the six-minute probe budget.
CPython 3.13.14 could use Zipline wheels but required a bcolz source build; that
installation did not finish within three minutes. These timeouts are operational
evidence, not proof that compiling on those interpreters is impossible.

The independently locked process uses CPython 3.12.13, Zipline 3.1.1,
bcolz-zipline 1.2.10, pandas 2.3.3, NumPy 2.5.2 and SciPy 1.18.1 wheels.
Actual imports succeeded. bcolz 1.2.10 imports `pkg_resources`; pin setuptools
80.9.0 explicitly. These upstream deprecations remain visible. Future upgrades
require the same behavioral and byte-identity gates, not removal of warnings.
`python/zipline_validation` is excluded from the primary uv workspace and uses
`uv run --isolated`; no extra persistent environment is created. No trading,
network data download or credentials are needed by the validator.

## Independent calculation and common dependencies

The primary exporter reconstructs the factor and all seven ledger artifacts,
verifies their actual frozen code/data/policy provenance, then exports causal
signals, eligibility, raw opening/closing quotes, time-resolved availability,
dated fees and declared corporate actions. Primary targets/orders/costs/NAV are
separate comparison objects and are never numerical inputs to the independent
engine. Resolved PIT selection and raw data remain disclosed common dependencies;
this is not an independent data vendor or independent factor AST implementation.

The separate process independently ranks stable IDs, sizes whole lots, applies
capacity, borrow and margin constraints, and calculates dated costs using exact
rational arithmetic. Zipline's actual `SimulationBlotter`, transaction creation,
commission application, `Ledger` and position tracker own filled holdings,
transaction cash and native mark-to-market NAV. The independently generated
execution instructions enter its slippage/commission extension points. Every
filled quantity is checked against the resulting native position. Exact cash
decisions are checked against native cash at every closing observation.

This is an event-driven use of Zipline finance components, not a claim to run
unmodified `TradingAlgorithm` defaults. Default daily bars do not implement the
primary next-opening dissemination clock. The adapter supplies each observed
opening to the real blotter, sells before buys at the same timestamp, expires
unfilled orders, and never uses a later opening to fund an earlier order.

## Explicit economic bridges

Zipline's default split handling floors negative fractional holdings and pays
cost-basis residual cash immediately. Its native NAV also omits unpaid dividend
claims. Applying either default silently would test a different strategy.

Use pinned, small adapter hooks for:

- rational whole-share splits with truncation toward zero and explicit delayed
  cash-in-lieu claims;
- signed dividend/delisting entitlements, final cash retirement and payment;
- economic internal financing/borrow flows, never external capital changes;
- observed raw marks through the native position tracker.

Retain every adjustment in the independent costs ledger. The bridge artifact
shows native Zipline cash/NAV, unpaid signed claims and economic NAV per session.
Economic returns use successive adjusted NAV ratios; neither delta-NAV nor
Zipline's claim-excluding native return silently replaces them. No terminal
claim payment, missing borrow availability or liquidation is invented.

Only the two reviewed primary profiles are accepted. Unsupported event kinds,
combined events, future data, missing held marks, unresolved fractional payment,
incomplete exchange sessions and invalid borrow/margin conditions fail closed.
No new portfolio policy, fee default or market-data coverage is inferred.

## Comparison, provenance and limits

Compare all seven ledger schemas, row counts, ordering, IDs, timestamps,
statuses, missingness and values. IDs, quantities and statuses are exact. Fixed
absolute tolerances are 5e-9 USD for prices, 1e-5 USD for monetary totals and
1e-12 for returns; relative tolerance is zero. Zipline uses float arithmetic;
native prices with ULP above 1e-9 USD, dollar totals with ULP above 1e-6 USD or cash error above
1e-5 USD are unavailable, not silently accepted by widening tolerances. Exact
rational decisions avoid one-share boundary changes due solely to float NAV.

Emit accepted/rejected/unavailable diagnostics, inspectable differences and an
immutable receipt. Cap the detail list but preserve total and omitted counts.
Record actual interpreter, adapter, numerical dependency and calendar bytes.
Ephemeral uv environments hardlink cached dependencies. Linking/unlinking can
change inode ctime without changing bytes; strict source reads therefore retry
that read race at most twice. Each attempt still checks all original metadata,
and the full source snapshot is re-hashed before publication. CAS evidence reads
retain immediate failure and are not retried or repaired by this exception.
Bound cells, files, output, deadlines and clock progression. Recheck all inputs
and build bytes before receipt-last publication; read-only replay recomputes
and verifies without repair. Interrupted runs may leave unreferenced immutable
objects, never a final success receipt. Such receipts carry no admission authority.

## Acceptance and rollback

Hand-calculated tests cover next opening, causal ordering, delayed dividend
claims, long and short fractional splits, cash delisting, dated SEC/TAF fees,
capacity/impact, margin, borrow recalls and weekend financing. Actual installed
primary-export/Zipline subprocesses must reconcile both profiles and replay
byte-identically. Wrong fills/costs/returns, corrupted artifacts, changed builds,
interruption, clock regression, unsupported precision and policy drift must deny.

Disable `zipline-prepare`/`loop-zipline run`, or revert this task commit, to stop
new writes. Retain input objects, native/economic ledgers and all old receipts.
No database schema, production connection, holdout unlock or factor admission
changes. Source changes invalidate current evidence rather than rewriting it.

## Primary references

- [Zipline Reloaded releases and wheels](https://pypi.org/project/zipline-reloaded/3.1.1/)
- [Zipline SimulationBlotter](https://github.com/stefan-jansen/zipline-reloaded/blob/main/src/zipline/finance/blotter/simulation_blotter.py)
- [Zipline ledger](https://github.com/stefan-jansen/zipline-reloaded/blob/main/src/zipline/finance/ledger.py)
- [Zipline position/split semantics](https://github.com/stefan-jansen/zipline-reloaded/blob/main/src/zipline/finance/position.py)
