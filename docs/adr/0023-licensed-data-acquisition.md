# ADR 0023: Licensed source acquisition with verifiable local receipts

- Status: Proposed; implementation and acceptance in progress
- Date: 2026-09-14
- Owner: hojiahao

## Requirement

Phase 5 unit 3 adds executable Sharadar acquisition and optional WRDS/Databento
paths. Downloads require explicit credentials and a currently valid local
license declaration. A successful request is evidence of that request's access,
not proof of redistribution rights, complete coverage or historical PIT quality.

## Decision

Extend the existing Python data package, private content-addressed cache and
bounded transport. Add no service, database table or Agent tool. A separate
licensed acquisition configuration and receipt keep public development records
backward compatible. The command validates a digest-pinned, private data-owner
license declaration before connecting. The declaration specifies source,
datasets, date scope, expiry and permission for internal research/local storage.
Current Sharadar ticker metadata additionally requires explicit reference-data
permission; historical date scope does not imply that permission.
Only declared prepaid subscription access is accepted; new purchases, metered
downloads and automatic subscription/ISIN allocation are unavailable. This is an
operator declaration checked against the request, not a vendor-signed license.

Use Nasdaq Data Link's fixed Sharadar Tables API routes for SEP, SF1, TICKERS and
ACTIONS. Use explicit date filters and projections, check named columns rather
than assuming their order, follow bounded cursor pagination to its terminal
page and reject duplicates or inconsistent schemas. Install the requested
official Nasdaq SDK and verify request encoding against its option converter;
raw HTTP streaming retains exact response bytes and enforces the shared budgets.
API keys travel only in the authenticated request and are removed from receipt
URLs. No response-driven redirect or bulk export URL is followed.

The three source paths use source-specific staging schemas, not the authorized
raw-bar schema. Nullable corporate-action key fields use a canonical composite
key digest; conflicting values for the same key fail closed. Permanent and SIC
identifiers preserve textual values, including leading zeroes. Original source
representations remain in the raw capture.

The optional WRDS path uses its documented PostgreSQL endpoint through Psycopg
3, which supports the project's Python 3.14 toolchain. It does not use the
application database. Fixed, parameterized read-only queries cover explicitly
versioned CRSP CIZ daily and Compustat quarterly records. Bound connection,
transaction, row and byte work; reject missing table/column permissions rather
than guessing another schema. Keep database numeric values as exact text.
No arbitrary SQL, interactive password prompt or ambient database credentials.

The optional Databento path accesses reference security-master/corporate-action
records through its documented HTTP protocol and pinned SDK contract. Explicit
dates, stable listing identifiers and `allocate_isins=false` prevent an access
probe from allocating new subscription slots. Allocation filtering can omit
rows: an empty result is not proof that no corporate event or listing exists.

Preserve native source columns and identifier namespaces in validated canonical
tables. Do not convert Sharadar split-adjusted OHLCV into raw execution bars or
infer unadjusted volume; keep closeunadj and closeadj separately. SF1 AR dimensions
retain datekey/reportperiod/calendardate/lastupdated without treating a date-only
field as a verified dissemination timestamp. CRSP CIZ returns already include
delisting observations; do not join a legacy dlret again. Compustat's currently
available fundq revisions and rdq are not automatically historical PIT facts.
Databento record/effective clocks and cancellation/revision fields stay intact.

Raw captures, normalized tables, non-secret configuration and the license
declaration each have content identities; publish the completion receipt last.
Offline replay rechecks bytes, request scope, pagination, schema and normalized
values. Source caches remain private and unpartitioned, inaccessible to discovery
and Provider processes. Only unit 4 may construct time-filtered Parquet snapshots
and quality/lineage reports for the existing authorized artifact boundary.

## Acceptance And Recovery

Exercise actual CLI configuration/cache/replay and HTTP/SQL contracts with
invented data. Test missing/expired/wrong-scope licenses and credentials before
IO, authentication/entitlement failures, secret redaction, column reordering,
schema drift, pagination, exact decimals, adjusted-price semantics, duplicate
records, malformed identifiers/dates, budgets, cancellation and corrupt replay.
Test read-only WRDS queries against a disposable PostgreSQL fixture, never the
production application database. Live access stays unverified without the
relevant subscription and credentials; no licensed data enters Git.

Rollback disables the new acquisition commands/writers. Preserve immutable
source objects and successful receipts. Existing SEC/Alpaca readers and the
application database remain compatible; there is no destructive down-migration.

## Primary Sources

- [Nasdaq Tables pagination](https://docs.data.nasdaq.com/docs/in-depth-usage-1)
- [Sharadar SF1 filters and dimensions](https://data.nasdaq.com/databases/SF1)
- [Sharadar price field definitions](https://sharadar.com/prices)
- [Official Nasdaq Python SDK](https://github.com/Nasdaq/data-link-python)
- [WRDS connection implementation](https://github.com/wharton/wrds/blob/main/wrds/sql.py)
- [WRDS CRSP CIZ transition](https://wrds-www.wharton.upenn.edu/documents/2084/Webinar.pdf)
- [Psycopg 3](https://www.psycopg.org/psycopg3/docs/)
- [Databento reference API](https://databento.com/docs/api-reference-reference)
- [Databento allocation controls](https://databento.com/docs/release-notes)
