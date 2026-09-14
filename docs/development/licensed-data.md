# Licensed US data acquisition

`loop-research data-acquire` downloads an explicitly selected, bounded source
partition into a private content-addressed cache. `data-verify` reconstructs its
normalized tables from the cached source evidence without network access.
These data-owner commands do not expose a research RPC or an Agent tool, register
a snapshot, grant holdout access or claim a completed portfolio backtest.

## Available source paths

| Provider | Implemented request | Preserved semantics |
|---|---|---|
| Sharadar via Nasdaq Data Link | SEP, SF1, TICKERS and ACTIONS Tables API, named projections and cursor pagination | Split-adjusted OHLCV, separate unadjusted/fully adjusted closes, as-reported SF1 dates, native permanent identities and corporate-action values |
| WRDS | Fixed CRSP `crsp.stkdlysecuritydata` CIZ daily and `comp.fundq` quarterly SQL profiles | Native CRSP price/delisting flags and returns, exact database text projection, Compustat reporting format and currently supplied vintages |
| Databento | Reference `security_master.get_range` and `corporate_actions.get_range` | Stable listing/security/issuer namespaces, all returned revisions and cancellations, nanosecond source clocks, nested native fields |

There is no automatic vendor fallback. An unavailable table/column, missing
permission, unsupported response, repeated cursor, truncated budget or failed
validation aborts completion. The adapter does not purchase subscriptions,
initiate metered historical-market-data jobs or allocate new Databento ISINs.
Nasdaq bulk-export redirects are not followed. Databento uses its native POST
form protocol and uncompressed JSONL; its SDK's `pit` option is a local reduction
policy, so no invented `pit` wire parameter is sent and no latest-only reduction
is applied.

## Installation

Use the existing root uv workspace and Python 3.14.4:

```bash
./scripts/uv.sh sync --all-packages --all-groups --locked
```

Pinned dependencies include `nasdaq-data-link==1.0.4`, `databento==0.86.0`,
`psycopg[binary]==3.3.5`, and the existing `httpx==0.28.1`. The Nasdaq and Databento
request contracts are checked against their installed official SDKs. WRDS uses
Psycopg 3's async PostgreSQL protocol, allowing bounded read-only transactions
on Python 3.14 without an interactive WRDS login or another virtual environment.
The application PostgreSQL database is not used for source downloads.

## Rights and configuration

1. Obtain the supplier's actual entitlement for internal research and local
   storage. The implemented acquisition path accepts only already prepaid
   subscription access. A declaration cannot change the supplier's billing or
   redistribution terms.
2. Write an owner-only JSON license declaration outside tracked configuration
   (mode `0600` or `0400`). Use the shape below with the actual source, dataset
   scope and validity. The sample identifier grants no rights.
3. Run `sha256sum` on that file. Copy the matching provider TOML from `config/data`
   to private runtime configuration and replace `license_sha256` with
   `sha256:<the-exact-file-digest>`. Select dates and explicit ticker/PERMNO/GVKEY/
   listing IDs. A partition is limited to eight identifiers and 10,000 rows.
4. Inject the secret references through the runtime environment. Keep API keys
   and passwords out of TOML, command-line arguments, shell history and Git.
5. Create an absolute, canonical, owner-only cache directory (mode `0700`).

```json
{
  "schema": "loop.data-license/v1",
  "provider": "sharadar",
  "license_id": "replace-with-non-secret-contract-reference",
  "declared_by": "hojiahao",
  "datasets": ["SHARADAR/SEP", "SHARADAR/SF1", "SHARADAR/TICKERS", "SHARADAR/ACTIONS"],
  "valid_from": "2026-09-14T00:00:00Z",
  "expires_at": "2026-10-01T00:00:00Z",
  "data_start": "2005-01-01",
  "data_end": "2026-08-31",
  "purpose": "internal_research",
  "local_storage": true,
  "billing": "prepaid_subscription",
  "current_reference_metadata": true
}
```

`declared_by` identifies the local declaration, not a transport-authenticated
approval or a vendor signature. The file digest, owner/mode, source, datasets,
date range and validity are checked before connecting and again at completion.
TICKERS supplies current metadata that can describe dates outside the requested
price period; it additionally requires `current_reference_metadata=true`.
No private license declaration or subscribed rows should be committed to Git.

| Configuration example | Required environment references | License datasets |
|---|---|---|
| `config/data/sharadar.toml` | `LOOP_SHARADAR_API_KEY` | Selected `SHARADAR/SEP`, `SHARADAR/SF1`, `SHARADAR/TICKERS`, `SHARADAR/ACTIONS` |
| `config/data/wrds-crsp.toml` | `LOOP_WRDS_USERNAME`, `LOOP_WRDS_PASSWORD` | `crsp.stkdlysecuritydata` |
| `config/data/wrds-compustat.toml` | Same WRDS references | `comp.fundq` |
| `config/data/databento-reference.toml` | `LOOP_DATABENTO_API_KEY` | Selected `databento/security_master`, `databento/corporate_actions` |

WRDS fixes host `wrds-pgdata.wharton.upenn.edu`, port `9737`, database `wrds` and
`sslmode=require`, matching its documented connection endpoint. `require`
provides encryption; it does not establish hostname verification. No custom
DSN, SQL, `.pgpass`, interactive prompt or ambient `PG*` setting is accepted.
Each operation uses one read-only transaction with explicit connection,
statement, lock, inactivity, total-time, row and serialized-byte limits.

## Commands and receipts

After preparing the actual private files and credentials:

```bash
./scripts/uv-research.sh run --locked --offline --no-sync loop-research data-acquire \
  /absolute/private/sharadar.toml \
  --license /absolute/private/license.json \
  --store /absolute/private/cache

./scripts/uv-research.sh run --locked --offline --no-sync loop-research data-verify \
  --store /absolute/private/cache --receipt sha256:<receipt-digest>
```

Here `uv --offline` controls package resolution; `data-acquire` still contacts
the explicitly configured supplier. `data-verify` performs no network requests.
It needs no current API credential and checks the license against the original
operation clocks, so expiry does not erase historical evidence or authorize a
new download.

Successful output contains a receipt digest, provider, per-table row counts,
empty/records status and the explicit `licensed_source_unverified` quality.
Receipts reference original HTTP bodies (or labeled PostgreSQL text projections),
non-secret request configuration, license bytes and normalized native tables.
Nasdaq query API keys are removed from receipt URLs and normal HTTPX logs.
Exact key/password echoes in successful payloads are rejected before caching;
free-form error bodies and provider warnings are not logged. This is exact
credential-echo protection, not detection of every possible encoded secret.

Databento's valid zero-byte JSONL response uses `content=null` in its capture:
this means precisely SHA-256 of empty bytes, not a missing nonempty artifact.
The final receipt records that response and zero received data bytes. Empty or
filtered results cannot establish absent events or complete coverage. The generic
artifact publisher continues to reject zero-byte objects.

The receipt is published last through the existing atomic no-replace publisher.
Failure or cancellation before that point leaves no completion receipt; already
captured source evidence remains. Cancellation after publication can lose the
CLI acknowledgment: inspect/replay receipts before assuming the operation never
committed. Corrupt cache objects are not overwritten to make verification pass.

## Research limits and next integration

- These are selected identifier partitions, not a survivorship-complete universe.
  Current ticker metadata must not be projected backward. Permanent source IDs
  and history require validated joins before building a research security master.
- Sharadar's OHLCV is split-adjusted. It is not relabeled as raw execution data;
  unadjusted open/high/low/volume are not fabricated from rounded ratios.
- SF1 uses explicit ARQ/ARY/ART dimensions and preserves datekey, reportperiod,
  calendardate and lastupdated. A date-only filing key does not establish the
  intraday instant that information became available.
- CRSP CIZ includes delisting observations in its daily series; a legacy DLRET
  must not be applied again. Compustat's selected `INDL/STD/D/C` fundq data is a
  current vendor projection; RDQ alone is not verified historical availability.
- Databento retains source record/creation times and all received versions.
  `allocate_isins=false` may filter rows outside existing allocations. No fixed
  2005-present coverage or data-quality conclusion is inferred from the API name.
- All raw caches and native staging tables remain private, outside discovery
  and Provider mounts. They can contain metadata/revision dates outside a price
  partition. Unit 4 must build immutable, period-filtered Parquet snapshots,
  source lineage, calendar/PIT/coverage reports and registry bindings before
  exposure through the authorized research boundary.

All committed fixtures are invented contract data. Without licensed live
credentials these paths remain locally/contract verified, not vendor-live
verified. The earlier SEC public live acquisition is separate evidence.

## Verification and rollback

`just check/test/build/doctor` are the workspace gates. The full test gate and
research CI explicitly enable `LOOP_WRDS_TEST_POSTGRES=1`, using only the
project's disposable TLS PostgreSQL server and a uniquely created test database.
It tests real read-only SQL, scope/format selection, exact decimals, row limits,
timeout/cancellation and connection closure; the test database is removed
without forcing leaked connections closed. Isolated offline invocations without
that opt-in report the database cases as skipped, not passed.

To roll back, disable the new data-owner commands/writers or revert the unit's
implementation commit. Keep immutable objects/receipts and research audit
history. There is no application schema migration or destructive down-migration.
