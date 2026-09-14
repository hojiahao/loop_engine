# ADR 0024: Reproducible source snapshots and bounded synchronization

- Status: Proposed; implementation and acceptance in progress
- Date: 2026-09-14
- Owner: hojiahao

## Requirement

Phase 5 unit 4 turns completed acquisitions into usable immutable Parquet files,
with independently checked lineage, dates, coverage and explicit quality limits.
The initial historical observation boundary includes 2026-08-31. A date label,
successful download or paid subscription must not certify historical availability,
complete delisting coverage or permission to expose a holdout to discovery.

## Decision

Extend the existing Python CLI and private content-addressed store. Reuse the
five implemented source adapters, their receipt replayers and no-replace atomic
publisher. Add no service, database table, mutable checkpoint or arbitrary URL.
Declare the already locked PyArrow dependency directly in the research package.

`data-snapshot` verifies complete acquisition evidence before converting source
tables into bounded Parquet objects. Preserve native values as exact strings,
including separate price-adjustment fields and source revision clocks. Add typed
observation dates and explicit availability/ingestion clocks. Partition by the
fixed research periods, retain revisions and exclude observation dates outside
the requested interval. Current reference metadata uses its observation date;
it cannot be backfilled into a historical partition. Empty selections remain
explicit in the quality report.

Record schema/Parquet hashes, row counts, source receipt and normalized-object
identities, native semantics, calendar version/session digest, missing values,
selected-identifier coverage and unresolved quality requirements. A revision
creates new objects and a new manifest; existing references remain unchanged.
Same source receipts, settings and writer version produce identical objects.
Offline `data-validate` replays sources, recomputes selection/quality and compares
the actual Parquet schema and values. Verify file hashes before parsing, bound
footer/row/column/uncompressed-byte work and reject external Parquet column files.

These are private source snapshots, not automatically authorized factor panels.
Business dates and known times stay distinct: later observations/restatements
must never receive an earlier known time. Source snapshots can contain versions
known after their business date. They remain outside every research/Provider
mount, and the existing runtime rejects their distinct manifest schema. The
Phase 6 panel materializer must make the verified security/PIT/decision-time join
before the existing authenticated broker can deliver a development view. Real
protected backtests still require the separate Phase 7 gate and single-use grant.

`data-sync` executes an explicit bounded list of existing source requests,
validates the complete plan and aggregate worst-case budgets before network IO,
and publishes immutable progress after each completed receipt. Resume verifies
the plan and every completed receipt before skipping a request. No moving alias
or filename scan supplies authority. A crash after download but before publishing
progress may repeat that read; subscription-only requests cannot allocate new
paid resources. Cancellation or failure publishes no successful final snapshot.
Completed source objects/progress remain replayable. Each retry still requires
current credentials and rights for requests that have not completed.

## Acceptance And Recovery

Exercise all five sources with invented records, the actual installed commands,
Parquet readers, empty/all-null columns, exact decimals, revisions, date boundaries,
missing sessions, tampered sources/manifests/Parquet, replay, budgets and cancelled
or resumed synchronization. Reuse the existing 2/4/8-process and kill/restart CAS
tests; add snapshot-level independent writers and publication-failure recovery.
Verify the real retained SEC receipt locally without redownloading it. That run
does not establish market-price or licensed historical universe coverage.

Disable the new commands to roll back. Preserve source objects, immutable progress,
snapshots and research/audit history; no destructive down-migration is required.
Production-quality acceptance requires actual licensed data and evidence, and
cannot be closed by synthetic fixtures or a self-declared license.

## References

- [PyArrow Parquet writer](https://arrow.apache.org/docs/python/generated/pyarrow.parquet.write_table.html)
- [PyArrow bounded Parquet reader](https://arrow.apache.org/docs/python/generated/pyarrow.parquet.ParquetFile.html)
- ADR 0018-0023: existing artifact authority, operator execution and source acquisition.
