# Provenance and licensing register

## Source-code baseline

The repository has an `upstream` remote pointing to `bs763/loop_engine` and an
`origin` remote pointing to `hojiahao/loop_engine`. At the Phase 0 baseline no
root `LICENSE`, `COPYING`, or `NOTICE` file was present in the tracked tree.

Absence of a license is not permission to relicense inherited code. Until the
rights are documented, the project will not add an MIT or Apache-2.0 claim over
the inherited tree. New visible product branding and maintainer metadata may be
changed, but Git authorship, third-party notices, and immutable audit evidence
must not be falsified or erased.

Before public release, every dependency must be recorded in an SBOM and checked
for license compatibility. External code may be used only through a compatible
license or an independently implemented protocol based on public documentation.

## Data-source classes

| Source | Intended level | Rights/quality gate |
|---|---|---|
| SEC EDGAR Company Facts | Development fundamentals | Obey fair-access policy; retain accession and raw filing metadata; use first-observed availability until historical dissemination is verified |
| Alpaca | Development market data | Probe and record IEX/SIP entitlement; never infer production quality from an account name |
| Sharadar/Nasdaq Data Link | Production candidate | Requires an active subscription and permitted local research storage/use |
| WRDS CRSP/Compustat | Institutional candidate | Requires organizational or academic entitlement and compliance with export/use terms |
| Databento | Optional cross-check | Record dataset entitlement; current corporate-action PIT coverage is not a full 2005-present substitute |

Credentials grant access but do not grant redistribution rights. Raw licensed
data, derived rows that violate vendor terms, and credentials must remain out of
Git and public release artifacts.

## Calendar Dependency

The initial date-only NYSE adapter uses `exchange-calendars==4.13.2` through its
public Python API; it does not copy or relabel upstream implementation code.
The [published package metadata](https://pypi.org/project/exchange-calendars/4.13.2/)
declares Apache-2.0. Installed copyright/license notices are preserved. Its
transitive dependencies remain pinned in `uv.lock` and subject to the complete
release SBOM/license gate. Calendar rules do not grant rights to market data or
prove point-in-time knowledge of exceptional future closures.

## Release gates

- A report states its exact data source, snapshot, entitlement class, and known
  limitations.
- Development data can produce only development-labelled results.
- Production claims require inactive/delisted coverage, point-in-time fields,
  corporate actions, and a documented right to use the data.
- Brand scans may replace obsolete presentation labels but must allow legal,
  historical, provider, and citation references.

## Development Ingestion Dependencies

Phase 5 unit 2 installs `alpaca-py==0.44.0` (Apache-2.0) and `httpx==0.28.1`
(BSD-3-Clause) in the existing root Python 3.14.4 environment. These declarations
were checked against installed distribution metadata and the
[Alpaca package](https://pypi.org/project/alpaca-py/0.44.0/) and
[HTTPX package](https://pypi.org/project/httpx/0.28.1/) records. Their published
Python APIs are used without copying/relabeling upstream implementations.
Installed license notices remain intact; all new transitive packages are pinned
in `uv.lock` and remain subject to the release SBOM and license compatibility gate.

SDK installation grants no market-data entitlement. SEC/Alpaca cache data belongs
in a private ignored runtime directory, never in fixtures or public Git. The
committed development HTTP fixtures are explicitly invented data. A successful
API request proves only that the particular request was accepted at that time;
it does not grant redistribution rights or certify historical PIT coverage.

## Licensed Acquisition Dependencies

Phase 5 unit 3 installs `nasdaq-data-link==1.0.4` (MIT), `databento==0.86.0`
(Apache-2.0) and `psycopg[binary]==3.3.5` (LGPL-3.0-only for Psycopg). Installed
distribution metadata supplies these declarations; original notices are retained.
The binary Psycopg distribution bundles native libraries with their own notices,
which must also appear in the release SBOM/license review. The inherited project's
licensing question is unchanged; installing a library does not relicense the tree.

The executable adapter uses official request contracts and a fixed read-only
PostgreSQL projection. It does not copy or relabel supplier implementation code.
All additional dependencies and hashes are pinned in `uv.lock`. Python 3.14.4 is
still the single primary workspace interpreter; there is no new virtual environment.

Licensed downloads require a private, digest-pinned local declaration of active
prepaid internal-research/storage rights. That declaration is not supplier-signed
entitlement proof. API acceptance verifies the specific request, while production
data quality and redistribution remain separately gated. Current Sharadar metadata
requires explicit permission in addition to historical date scope. Databento new
ISIN allocation is disabled. No subscribed vendor live request was made as part
of implementation without credentials; test records are explicitly invented.

Sources: [Nasdaq SDK](https://pypi.org/project/Nasdaq-Data-Link/1.0.4/),
[Databento SDK](https://pypi.org/project/databento/0.86.0/),
[Psycopg](https://pypi.org/project/psycopg/3.3.5/).
