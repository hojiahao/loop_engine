# ADR 0016: Durable IS window perturbation

- Status: Accepted for the internal command and installed worker; production denied
- Date: 2026-09-10
- Owner: hojiahao

## Requirement And Scope

Phase 4 delivery unit 1 must use registered Sharpe history, preserve random and
optimizer state across restart, and actually exclude recorded failed candidates.
The legacy perturbator's empty-history no-op is not a usable implementation.

Implement one bounded, fixed single-window family. Rust owns authority, evidence,
PostgreSQL and atomic decisions. The installed Python research module owns the
numerical transition. One Protobuf request/response crosses a bounded subprocess
boundary; no database connection, dataset, credential or holdout capability does.
This is not a new service, arbitrary-code tool, factor-admission path, Bayesian
optimizer or claim that the complete discovery scheduler is available.

## Evidence And Identity

`AdvancePerturbation` names an immutable context, source job, expected revision,
idempotency key and deadline. It accepts no caller-provided scores or random state.
The existing `BacktestPolicy` must resolve a `PerturbationSpace` proving:

- All data is IS, not development validation or either historical holdout.
- The 2-64 canonical FactorSpec IDs differ only at one declared window parameter;
  direction and all research/execution/cost policies remain fixed.
- Data references, six-component provenance, backtest seed and worker build are
  pinned. The independent perturbation random seed is also fixed.
- Windows are unique, ascending integers in 1-4096. Reopening a context with
  changed space content is an error, not a silent optimizer reset.

Only a current, registered primary-backtester result supplies a score. The exact
metric contract is `net_sharpe`, unit `dimensionless`, estimator
`sample_std_ddof1_sqrt252_zero_rf.v1`: sqrt(252) times the mean net daily simple
return divided by its sample standard deviation (ddof=1), with zero reference
rate. The result producer must omit this metric when undefined; zero volatility
is not assigned an artificial score. ADR 0013's resolver remains responsible
for verifying computation evidence, not just the metric label. Phase 7 has not
yet implemented the real market-data producer of this metric.

Scores must be finite and within [-1e6, 1e6]; nonzero exact decimals that underflow
float64 are refused. A source job enters history once. Reusing its ID with a
different observation fails. A deterministic coverage/filter/performance
rejection may trigger another proposal without creating a fake zero Sharpe.
Operational failures, cancellation and protected jobs cannot become observations.

For every candidate, the command queries ADR 0015's indexed frozen-context
rejection memory. No caller-supplied failed set can bypass it. All historical
rejections remain recorded; this optimizer only reuses the context-independent
codes defined there. Successful observations and previous proposals are excluded
as well. Proposal reservation is not a negative research result: retrying an
operationally failed evaluation belongs to its job lineage, not a new proposal.

## Numerical State

`window.ema-gradient-pcg64.v1` uses NumPy PCG64 with the 32-byte seed interpreted
as an unsigned big-endian integer. Store the count of consumed raw uint64 draws;
reconstruct the same generator and advance it on restart. Bounded rejection
sampling chooses uniformly without modulo bias, including tie-breaking.

Cold start explores an eligible window instead of returning the source window.
With history, compute a local Gaussian-weighted least-squares slope with window
bandwidth 5, then update first and second moments with decay 0.7. Propose the
eligible window closest to `current + 2*m/(sqrt(v)+1e-8)`. Zero/undefined slope
falls back to exploration. The algorithm, constants and RNG family are versioned;
they are research heuristics, not evidence of better investment performance.

The state preserves up to 1024 unique job observations, moments, seed, draw count
and ordered proposed IDs. Budgets never silently truncate history: 64 candidates,
1e9 cumulative draws, 32 draws per choice, and 1 MiB wire envelopes. An exhausted
family returns an explicit reason and no candidate without consuming randomness.
Python serialization is transport, not a canonical factor identity.

## Atomicity And Recovery

1. Under the existing bounded PostgreSQL ledger lock, authorize the principal,
   resolve fresh source evidence and family, verify state against its last
   immutable receipt, check replay/CAS, and resolve failure memory.
2. Release the transaction before invoking the real numerical worker. Its
   absolute executable is service-configured, never supplied by an Agent.
   Clear inherited environment, pin numerical thread counts, use Python isolated
   mode, cap stdin/stdout and kill on cancellation or the 10-second timeout.
   This process boundary is not an OS security sandbox; task 4 owns deployment
   isolation, and task 3 owns trusted build/artifact verification.
3. Reacquire the ledger lock, re-resolve permission/evidence and check revision.
   A newly rejected selected candidate blocks commit. Verify the worker did not
   change history, seed, immutable settings or proposal exclusions.
4. Commit the revision-fenced state, immutable command receipt, audit event and
   clock watermark atomically. Return the proposal only after commit.

Migration 8 adds one table. The state references its latest command receipt via
a deferred foreign key; reads verify byte checksums and the receipt/state binding.
SQL triggers forbid deletion, context/space changes, skipped revisions and time
regression. Immutable receipts preserve every accepted state, so replacing the
current projection does not discard history. Checksums are corruption detection,
not defense against an administrator rewriting all evidence.

An identical retry revalidates authority and freshness but returns the original
receipt without executing Python, advancing RNG or appending audit. A changed
request under the same key conflicts. Different concurrent commands use CAS;
only the winner commits. Calculations can be repeated after a crash before
commit, but externally returned proposals are commit-once, not exactly-once
arbitrary external effects. Overall command deadline is at most 30 seconds.

## Remaining Gates And Rollback

Production resolvers remain default-deny. Tests use real PostgreSQL and the
installed Python worker, but explicitly fabricated IS result metadata. They
do not prove market-data quality, canonical candidate derivation or profitability.
Task 3 must implement trusted family/result manifests; task 4 must bind actual
transport identities and storage isolation; task 5 must bind authorized AST
evaluation. Phase 11 will consume these commands in the bounded discovery loop.

Migration 8 has not been applied to production. Before deployment, reverting
this delivery restores the previous implementation. After deployment, disable
the perturbation resolver/writer, retain migration 8, states, receipts and audit,
and use a schema-aware compatibility build or forward fix. Do not drop the table,
delete migration records or restore an older snapshot over committed research.
Existing migration files and the A-share archive are unchanged.
