# Causal development factor panels

`panel-build` converts byte-backed OHLCV observations and explicit security
histories into the existing authorized factor worker's input. It produces raw
values, eligibility, a complete XNYS session grid and immutable lineage. It does
not search for factors, register jobs, admit factors or calculate portfolio returns.
ADR 0026 records the design and rollback boundary.

## Inputs and authority

Use two existing, absolute, canonical directories owned by the data administrator
with mode `0700`. The private source store contains the capture and its original
source bytes. The development output store contains only derived panel artifacts.
The directories must be separate and must not contain one another. Neither path
is supplied by an Agent or accepted through a runtime RPC.

The request format is `loop.panel-build-request/v1`; the complete executable
synthetic example is `fixtures/market/panels/request.json`. It freezes:

- The PIT capture's SHA-256 and exact byte size.
- An optional verified Phase 5 source-snapshot reference.
- Sorted, unique stable security IDs and raw fields: `market.open`, `market.high`,
  `market.low`, `market.close`, `market.volume`.
- An explicit warmup start and an evaluation range wholly within IS 2007–2016
  or development 2017–2020. Warmup starts no earlier than 2005-01-01.
- A delay of 0–7,200,000 milliseconds after the actual XNYS scheduled close;
  the default is five minutes. Early closes and daylight saving are respected.

Synthetic captures use the existing `loop.pit-input/v1` schema and may contain
invented security histories and bars. Every declared raw-source digest must
resolve to bytes in the private store. Fundamentals are outside this OHLCV unit
and are rejected, as is `market.adjusted_close`. An adjustment policy must be
implemented explicitly before adjusted prices can become inputs.

Public-development captures contain security histories only. Price records are
loaded exclusively by replaying the exact SEC/Alpaca acquisitions underlying the
verified source snapshot. Caller-supplied public price records are rejected.
The source snapshot and every acquisition's selected business-date range must
fit inside the requested warmup-through-evaluation interval. Broader ranges are
rejected before their source records are replayed. SEC fundamentals do not become
OHLCV fields. Licensed sources remain subject to the deferred production gate.

An explicit, source-backed history is still a data-owner declaration; this path
does not certify vendor historical universe coverage. A current Alpaca asset
response cannot establish a past listing or instrument type. Similarly, an old
bar fetched today retains its first-observed timestamp. It cannot be backdated
into a past decision. A full-local-day bar is unavailable before that interval
ends. These limitations can legitimately produce a public-development panel
with zero eligible or observed rows; that is evidence of insufficient inputs,
not a successful historical strategy test.

## Construction and verification

For an existing private capture and request, run from the repository root:

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research panel-build \
  /absolute/private/panel-request.json \
  --sources /absolute/private/source-cache --store /absolute/development-cas

./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research panel-validate \
  --receipt sha256:CONSTRUCTION_RECEIPT_DIGEST \
  --sources /absolute/private/source-cache --store /absolute/development-cas
```

Replace the receipt placeholder with the first command's `receipt.sha256`.
No network access, API key or subscription is needed to build from completed
source captures. `panel-validate` reconstructs the selection and all derived
bytes, compares the source identity and output references, and writes nothing.

Each session/security pair remains present, even when no bar exists. The latest
visible security state determines default eligibility: listed ordinary common
stock on XNYS, XNAS or XASE. Expired/delisted states never revive an earlier active
version. Ambiguous visible ticker assignments fail. Missing prices remain empty;
there is no forward fill. Only USD raw bars covering the regular session and
known by the decision may supply values. Visibility comparisons retain
microseconds; exported millisecond knowledge times round upward. The fixed
capture time is the ingestion cutoff and must reach the final decision exactly.

Decimal prices are converted explicitly to finite binary64 for the numerical
worker. Integer volume must survive that conversion exactly. Coverage reports
include both all rows and evaluation-only counts so warmup cannot inflate the
latter. Zero coverage is reported, not automatically admitted.

Outputs reuse `loop.factor-panel/v1`, its CSV, `loop.trading-calendar/v1` and
`loop.development-dataset/v1`. The private `loop.panel-build-receipt/v1` binds the
request, builder semantic version, actual installed research-source identity,
calendar package version, output identities and coverage counts. The dataset
snapshot ID also binds those construction inputs. It does not expose private
capture paths or raw source references to the worker. Source identity matches
the source component used by `build-manifests`; it is not an environment or
whole-host attestation. The authorized worker separately checks its complete
frozen source/environment provenance before and after execution.

The returned `dataset` and `calendar` references can be pinned in the existing
research context described in `factor-evaluation.md`. Generating these files
does not create a runtime identity, job, lease or capability. The existing
resolver and artifact broker still verify the references and expose only the
authorized panel and CSV in a read-only worker view. The integration tests use
the installed panel command before real TLS/PostgreSQL/lease-controlled execution.

## Limits and recovery

Construction is bounded to 10,000 capture records, 512 capture-referenced raw
files, 2 million session/security cells, 50 million selection work units and a
cooperative 180-second deadline. The capture's raw-file set and the public
snapshot's acquisition-response set each have a separate 512 MiB byte budget;
replay may read an object more than once. Individual metadata/CSV and source-
replay limits also apply. Large universes or long captures must be split into
explicit selections; this is not yet a full-market throughput SLA.

Derived objects are published without replacement, then the final construction
receipt is published in the private source store. Cancellation or failure can
leave valid intermediate objects. Only a final receipt signals completion; a
lost acknowledgment is recovered by rerunning the identical request. No mutable
`latest` pointer is maintained. Validation of old receipts requires the original
locked builder version if the installed source identity changes.

Rollback disables these administrative writers and reverts this task's code.
Retain source snapshots, captures, panels, receipts, completed jobs and audit
history. No database migration or destructive cleanup is required. Every report
remains `production_eligible: false`; protected samples are unsupported here.
