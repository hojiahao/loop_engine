# ADR 0021: Point-in-time security and observation records

- Status: Accepted
- Date: 2026-09-13
- Owner: hojiahao

## Requirement

Phase 5 unit 1 must resolve the security and information that existed at an
explicit historical decision time. A current ticker list cannot be projected
backwards, a later restatement cannot replace an earlier visible fact, and an
issuer identifier cannot silently identify one of several share classes.

## Decision

Keep immutable, vendor-neutral records and queries in the Python data package.
Reuse the existing snapshot/reference boundary; this unit adds no service,
PostgreSQL table, Protobuf payload, market-data download or new dependency.
An executable local diagnostic reads a bounded file, validates every record,
queries an explicit market/knowledge time and emits a checksummed development
report. Raw vendor adapters and Parquet publication are the next delivery units.

Each security version carries a stable security ID, separate issuer ID, ticker,
listing venue, instrument classification and listing status. Versions carry
effective, known and ingested timestamps, with optional exclusive effectiveness
end. History is append-only in each immutable input. Later effective events
supersede earlier events; revisions at the same effective instant are ordered
by their known time. Conflicting ties and ambiguous ticker mappings fail closed.
An expired or delisted latest state must not resurrect an earlier active state.

Market observations preserve raw OHLCV and their interval end. Fundamental
observations preserve issuer, concept, unit, fiscal period, filing reference and
exact decimal value. Corrections create new visible versions. Neither current
prices nor restated facts are silently substituted for historical observations.
The source declaration names the dataset, revision, raw-content digest and
availability basis. First-observed data may not claim an earlier known time.
Revisions of a logical observation must have one explicit source dataset;
bar revisions also retain currency. Different vendor feeds require an explicit
reconciliation policy rather than a latest-timestamp substitution.

Queries use distinct market time, public-information cutoff and an ingestion
cutoff fixed by the captured input. Historical public knowledge can precede
today's ingestion; the two clocks are not conflated. Unknown or absent records
remain absent. Only explicit synthetic/public-development inputs are accepted
by this diagnostic; declarations are not proof of licensed, survivorship-aware
or complete point-in-time coverage.

## Evidence And Recovery

Require synthetic histories covering ticker reuse, multiple share classes,
delisting, delayed reports, later restatements, unavailable revisions, ambiguous
ties, bad timestamps, invalid prices, exact decimals and malformed input files.
Exercise the installed CLI against actual files. Verify deterministic query
output and that later invisible records cannot change an earlier decision.

Rollback removes/disables the new diagnostic. Retain source inputs and their
checksums; this unit has no destructive migration or production writer.

Implementation `523a736` is pushed. Local check/test/build/doctor and all seven
jobs in CI run `34749353894` pass. This accepts the local development query,
not production-data coverage or full Phase 5 completion.

## Source Boundaries

The [SEC API documentation](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)
describes CIK as a filer identity, company submission metadata and aggregated
XBRL facts. This supports separating issuers from securities and retaining
filing-level availability evidence. It does not establish a complete historical
security master.

Alpaca's [assets endpoint](https://docs.alpaca.markets/us/reference/get-v2-assets-1)
lists assets available through that provider. Its response must not be treated
as a survivorship-complete historical universe without additional evidence.
The concrete SEC/Alpaca adapters and entitlement checks belong to unit 2.
