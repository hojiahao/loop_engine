# ADR 0022: Bounded SEC and Alpaca development ingestion

- Status: Accepted
- Date: 2026-09-13
- Owner: hojiahao

Implementation `cc462da` is pushed. CI run `34765587180` passes all seven jobs,
including isolated research dependencies, unified commands and the clean
DaoCloud container. This accepts the bounded development adapter/cache workflow;
it does not certify Alpaca live access or production historical data quality.

## Requirement

Phase 5 unit 2 installs the requested development data dependencies and delivers
an executable ingestion path: explicit configuration, bounded provider requests,
validated observations, immutable raw/normalized cache objects and a receipt.
It must expose authentication/entitlement failures without inventing data,
silently switching feeds or claiming a complete historical security master.

## Decision

Keep the adapters in the existing Python data package. Install and lock
`alpaca-py` and `httpx` in the existing root Python 3.14.4 workspace. Reuse official
Alpaca request models for wire parameters. Use a small shared streaming HTTP
transport so original JSON bytes, exact decimal parsing, response-size limits,
deadlines and bounded retries have one enforced path. This adds no service or
metadata database table and does not expose order/account mutation endpoints.

Only explicit SEC/Alpaca HTTPS data routes are available. Redirects, arbitrary
hosts, implicit credential lookup, response-driven URLs, unbounded pagination
and feed fallback are denied. Credentials are resolved from named environment
references, never persisted or logged. Request/page/byte/time budgets are finite;
SEC requests are sequential and throttled below its published per-user limit.
The operator remains responsible for aggregate access across multiple hosts.

The local ingestion command reuses the private no-overwrite content-addressed
publisher from build identity. Original successful response bytes and normalized
records have independent SHA-256 references. A final receipt identifies the
configuration, source, declared feed, observation clocks, counts, limitations
and objects. Validation/download failure leaves no success receipt; immutable
source evidence is not silently deleted. Receipt publication is the local commit
point: interruption after it can prevent CLI acknowledgment, so retry/replay must
inspect the cache instead of assuming nothing committed. Parquet snapshot assembly, licensed coverage, registry
admission and large-data synchronization remain units 3/4.

## Provider Semantics

SEC ingestion preserves CIK, accession, concept, unit and fiscal period. The
company-facts API aggregates filing vintages, so `filed` dates and period ends
are not substituted for public-availability timestamps. The development path
uses conservative first-observed availability for values without independently
verified historical dissemination evidence. It preserves raw filing metadata
for later verification and selects an unambiguous latest observed value per
concept/unit/period; conflicting same-date vintages fail closed. Such output
cannot reconstruct earlier historical knowledge or certify PIT fundamentals.
SEC Company Facts/submissions responses can contain dates outside the configured
normalization range. The unpartitioned raw cache is data-owner-only and must never
be mounted into discovery/provider deployments or registered as an IS-only
artifact. Later snapshot assembly must keep raw lineage separate from the
validated, period-filtered rows exposed through the authorized data boundary.

Alpaca requests explicitly select `feed`, `adjustment=raw`, date bounds, sorting
and symbol-asof behavior. The adapter checks asset identifiers and records the
scope of current metadata instead of inferring an issuer or ordinary-common-stock
classification. IEX coverage is not labeled consolidated SIP. A successful
historical request proves access only for that feed/range; missing credentials,
401/403, rate limits and empty data are distinct statuses. Access to recent SIP
requires its own probe and is not inferred from older data.

Daily bars use the provider's New York day labels and trade-condition rules.
RawBar validation gains an explicit next-local-midnight exclusive end case for
a complete daily aggregation. The source's complete response is first observed
at ingestion; a midnight label is not a time when the day's volume was known.
Existing within-day records keep their interpretation. Corporate actions,
delisting returns, historical issuer mapping and production quality remain
unresolved until their dedicated adapters and quality gates provide evidence.

## Acceptance And Recovery

Exercise the installed CLI against actual files and controlled HTTP fixtures.
Check SDK request encoding, explicit dates/feed/asof, multi-page completeness,
decimal preservation, malformed records, duplicates, inconsistent identifiers,
429/retry bounds, redirects, oversized/compressed responses, clock regression,
cancellation/deadlines, secret redaction and cache replay/corruption. Install
the declared dependencies and pass isolated-environment tests. A bounded public
SEC live probe is bounded by the same transport; Alpaca live verification requires credentials.
Offline success must not be reported as live verification.

The shared publisher is also exercised by 2/4/8 independent OS writers and by
kill/restart immediately before and after its atomic no-replace link. A hard kill
can leave a unique `.loop-build-*` temporary hard link; it is never a valid object
or completion receipt. Clean such files only in a stopped, exclusively owned
cache or a disposed test directory, not while another writer may be active.

Rollback disables the ingestion command and new writers. Preserve successful
cache objects/receipts and existing audit history. No destructive down-migration
or production database change is required.

## Primary Sources

- [SEC data APIs](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)
- [SEC developer resources and fair access](https://www.sec.gov/about/developer-resources)
- [Alpaca historical bars](https://docs.alpaca.markets/us/reference/stockbars)
- [Alpaca feed and aggregation rules](https://docs.alpaca.markets/us/docs/market-data-faq)
- [Official Alpaca request models](https://alpaca.markets/sdks/python/api_reference/data/stock/requests.html)
