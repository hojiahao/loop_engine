# ADR 0026: Causal development panels from captured security histories

- Status: Implemented; local gates pass; publication gates pending
- Date: 2026-09-14
- Owner: hojiahao

## Requirement

The owner defers paid subscriptions and authorizes Phase 6 development with
synthetic/public-development evidence. The Phase 5 licensed historical coverage
gate remains open. Unit 1 must turn captured records into the exact input of the
existing authorized numerical worker, without manually fabricated panel CSVs,
current-universe backfill or hindsight revisions.

## Decision

Add bounded administrative `panel-build` and `panel-validate` commands to the
existing research CLI. Reuse the PIT records/query rules, pinned XNYS calendar,
private content-addressed reader/publisher and existing panel/dataset formats.
No new service, database table, RPC or dependency is required.

A strict request pins a PIT capture, explicit sorted security IDs, raw OHLCV
fields, warmup/evaluation dates and a delay after the scheduled session close.
The capture is synthetic, or a public-development security history paired with
a verified Phase 5 source snapshot. In the latter case, price observations come
only from replayed development acquisitions in that snapshot; caller-supplied
bars are rejected. Security history remains an explicit data-owner declaration
backed by source bytes, not vendor-certified historical universe quality.

Resolve every requested session, including missing days. At each decision use
only public/ingestion-visible security versions and that session's latest visible
raw bar. Preserve delisted/expired/excluded states as ineligible cells and absent
prices as missing cells. Never revive an older active security state, alias ticker
identities, fill yesterday's price or move a first-observed timestamp backwards.
Volume conversion to binary64 must remain exact. Adjusted prices are unsupported
until a separate explicit corporate-action/adjustment policy exists.

Restrict both evaluation and source business-date selections to one IS/development
sample plus preceding warmup. Reject protected source snapshot ranges before
replaying their observations. Source stores remain separate from published
development stores and worker views. Whole source responses may contain broader
vintages; only the causally selected derived panel reaches the worker.

Publish the unchanged `loop.factor-panel/v1`, values CSV, calendar and development
dataset formats. A private construction receipt binds input references, builder
source identity, calendar dependency version and all derived objects. The dataset
snapshot ID binds the construction inputs without exposing private raw data.
Offline validation rebuilds and compares the complete output, with no publication.
The deployment must explicitly pin the returned dataset/calendar in its existing
research context; generating a panel grants no job, lease or holdout authority.

Bound input bytes, source records, cells, total selection work and elapsed time.
All error/cancellation paths leave no successful final receipt; prior immutable
objects survive. Retry of identical inputs reuses the same objects. A changed
builder identity requires a new receipt; existing worker provenance already
captures the entire research package and invalidates stale computations.

## Acceptance and recovery

Exercise missing sessions, delayed corrections, listing/delisting/ticker reuse,
expired intervals, excluded instruments, source/capture corruption, data-store
separation, unsupported adjusted fields, exact volume conversion, clock regression,
deadlines and deterministic replay. Test a real installed CLI and route its
generated dataset through the existing TLS/lease/PostgreSQL/Python evaluation
integration. Preserve the established publication/concurrency and numerical gates.

Rollback disables the new administrative writers and reverts this task's code.
Keep completed source captures, construction receipts, panels, jobs and audit
history. No destructive migration, production database operation, paid request,
subscription purchase or holdout unlock is part of this unit.
