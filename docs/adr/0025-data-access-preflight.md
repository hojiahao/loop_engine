# ADR 0025: Offline data access preflight and Phase 5 handoff

- Status: Accepted for offline access checks; live supplier acceptance remains open
- Date: 2026-09-14
- Owner: hojiahao

## Requirement

Finish the data-plane operating workflow before starting Phase 6, as requested
on 2026-09-14. A data owner must be able to identify missing credential references
and local license problems before starting a download. Batch synchronization
must check every remaining request before publishing progress or contacting a
supplier. A successful local check cannot assert live entitlement or historical
universe/PIT quality.

The existing batch checks do not reject a future date or credential material in
a later request's license until that request begins. Earlier requests can already
have downloaded data. An offline command must share production validation rather
than introduce an unrelated, passing diagnostic.

## Decision

Add `loop-research data-preflight` for existing single-source TOML files and
bounded sync plans. Reuse the existing strict request models, credential
validators, private license reader, license digest/scope checks and cache-directory
checks. Return a small structured report with reference names and static reasons,
never credential values, license bodies, URLs, paths or supplier response bodies.
The command makes no network requests, writes no files and creates no grants.

Use the same request preflight in `data-sync` before publishing its plan or
executing the remaining prefix. Validate complete historical dates, credential
formats, exact license bytes/scope/validity and sensitive configuration. Actual
acquisition rechecks its current credentials and license at execution/completion;
preflight is a diagnostic, not a reusable authorization token. Completed offline
resume remains possible without current credentials.

The CLI exits 0 only when local requirements pass, 3 for a valid configuration
with unmet local access requirements, and 2 for malformed or unsafe input. Its
report always identifies live access as unchecked and production eligibility as
false. It performs no subscription purchase or paid verification request.

No new service, database table, dependency, secret store or mutable credential
registry is needed. Supplier onboarding documentation must match the implemented
protocols: the Sharadar adapter uses Nasdaq Data Link credentials, not an
unrelated direct-API key. WRDS source credentials are separate from Loop Engine's
application PostgreSQL identity.

## Acceptance And Recovery

Exercise the installed CLI and real synchronization with missing/malformed
credentials, mismatched/expired licenses, unsafe file permissions, future ranges,
sensitive configuration, fully completed resume and every supported source.
Assert no network or publication on preflight failures. Verify all shipped
configuration examples and document live-vendor verification as unavailable
without credentials. Reuse existing full workspace and remote CI gates.

Rollback disables the new diagnostic and sync writer or reverts this delivery
unit. Preserve all source objects, receipts, snapshots and research/audit history.
There is no schema migration or destructive data cleanup.
