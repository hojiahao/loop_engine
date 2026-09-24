# ADR 0012: Research provenance integrity and freshness

- Status: Accepted for the Phase 4 metadata comparison boundary
- Date: 2026-09-09
- Owner: hojiahao

## Decision

Reuse the six required digests in `ResearchProvenanceFingerprint`: executable
source, operator registry, resolved configuration, data manifest, trading
calendar and execution environment. Do not hash Protobuf serialization or add a
second identity format for simple field equality. Validators own immutable
copies and check every component in stable protocol field order.

Metric evaluation distinguishes three inputs: the recorded result fingerprint,
the original run's independently resolved frozen fingerprint, and the currently
requested context resolved by the owning service. Check original-run integrity
first. A recorded/frozen mismatch is an integrity error, even if current inputs
match the erroneous result or the current context is unavailable.

A valid historical record is `current` only if all six digests match the
resolved current context. Otherwise it is `stale`, with every changed component,
or `unresolved` when the owner cannot resolve that context. The current-result
gate rejects both states. Never overwrite historical fingerprints or automatically
reinterpret old metrics under new inputs. Infrastructure failures and integrity
errors are not factor rejection.

The Rust, TypeScript and Python protocol packages implement the same assessment
and share `tests/contracts/provenance_vectors.tsv`. Existing job validation uses
the same fixed-width snapshot constructor instead of a separate field checker.
No Protobuf schema or previously published identity format changes.

## Remaining integration gates

This primitive is not authorization, worker attestation, a complete metric
manifest, or a production artifact registry. Equality of caller-supplied hashes
proves none of those. Owning services must resolve the immutable original run,
factor/specification, sample, seed, and checksummed artifact manifests before
comparison. The current context is explicit, not a global `latest` alias.

Phase 4 must still connect these checks to durable result registration and
current-result reads/exports, and verify reference invalidation across restarts.
Phase 5/6/7 must bind actual data, evaluator and accounting execution to these
resolved values. Numerical primitives and metadata comparisons alone cannot
close Phase 4 or validate the 23 stale A-share factors.
