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
| SEC EDGAR Company Facts | Development fundamentals | Obey SEC fair-access policy; retain accession and acceptance timestamps |
| Alpaca | Development market data | Probe and record IEX/SIP entitlement; never infer production quality from an account name |
| Sharadar/Nasdaq Data Link | Production candidate | Requires an active subscription and permitted local research storage/use |
| WRDS CRSP/Compustat | Institutional candidate | Requires organizational or academic entitlement and compliance with export/use terms |
| Databento | Optional cross-check | Record dataset entitlement; current corporate-action PIT coverage is not a full 2005-present substitute |

Credentials grant access but do not grant redistribution rights. Raw licensed
data, derived rows that violate vendor terms, and credentials must remain out of
Git and public release artifacts.

## Release gates

- A report states its exact data source, snapshot, entitlement class, and known
  limitations.
- Development data can produce only development-labelled results.
- Production claims require inactive/delisted coverage, point-in-time fields,
  corporate actions, and a documented right to use the data.
- Brand scans may replace obsolete presentation labels but must allow legal,
  historical, provider, and citation references.
