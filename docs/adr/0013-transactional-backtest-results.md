# ADR 0013: Transactional backtest result registration

- Status: Accepted for the internal persistence boundary; production resolution is pending
- Date: 2026-09-09
- Owner: hojiahao

## Decision

Successful development and holdout backtests must register one structured
`BacktestResult` in the same PostgreSQL transaction as the terminal job revision,
immutable command receipt and audit append. Other outcomes, including genuine
infrastructure failures and factor rejections, do not create successful metrics.
Large result series remain immutable artifact references outside PostgreSQL.

Migration 6 adds immutable `backtest_results`, unique job and backtest/engine
bindings, and deferred constraints on both sides of the job/result relationship.
It refuses deployment over pre-existing successful backtests without migrated
evidence. It does not fabricate their metadata. Existing published migrations
are unchanged; deployment remains separate from runtime startup.

The existing lease-fenced completion handler is the only insertion path. Its
server-owned `BacktestPolicy` must independently resolve the result manifest and
verify the frozen job, factor, sample, deterministic seed, engine and every
referenced artifact. These synchronous callbacks are bounded, pre-resolved
metadata operations, not a network client running under a database lock.
The default denies completion, even if ordinary job admission allows it.

Storage additionally verifies the six original-job fingerprints, protected
backtest identity, required result-series references, manifest/output binding,
bounded sorted unique metrics, exact finite decimals, and completion time.
The metadata envelope checksum detects corruption of stored bytes; it is not a
new semantic research identity or a hash of canonical Protobuf serialization.
Immutable replay verifies persisted result bindings without redispatch or a
second result insertion. A new current context does not rewrite an old receipt.

`BacktestRepository::current_backtest` authorizes a transport-authenticated
principal through the owning admission policy, independently checks
protected-period access when applicable, verifies
the stored record and resolved artifact evidence, and compares all six digests
with an explicitly named server-resolved current context. Missing context,
changed inputs and inconsistent original evidence fail closed. Reads do not
change history. This API is not an external export or approval endpoint.

## Remaining boundaries

Only test fixtures currently implement the result resolver. They are not real
BacktestSpec parsers or artifact registries. This checkpoint therefore does not
enable a production completion endpoint, attest numerical execution, authorize
holdout data, or close Phase 4. Production reference registries and capability
authentication remain required; Phase 5/6/7 must bind actual data, evaluator,
sample, seed and accounting execution to the resolved manifests. Export handlers
must use the same freshness gate and add their own permission and audit record.

The result is current only for the explicitly resolved immutable context, not
for an unqualified moving `latest`. A dashboard must show that context, and a
run must pin it. Unresolved infrastructure is never factor rejection.

## Required verification

Exercise registration and replay across restart, each stale component, absent
contexts, default denial, exact-decimal and artifact failures, lease expiry,
immutability, corruption, rollback after result insertion, and SQL-level bypass
attempts. Expand the existing 2/4/8 OS-process and kill/restart matrices to cover
result completion and persist verification evidence before publication.
