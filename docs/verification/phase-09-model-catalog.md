# Phase 9 unit 7: Model catalog publication and activation

Status: implementation, full TypeScript regression, `just check` and workspace
build passed on 2026-09-24. Commit `f9f05f5` is pushed; exact-commit CI
`35965098459` passed six jobs. The DaoCloud container failed the existing
`cancelled_descendant_dies` test, whose PID readiness race is addressed by the
platform acceptance task. Its corrected integrated gate remains required.
Phase 9 is not yet complete; no `main` merge is claimed.

## Requirement and design

ADR 0044 makes model metadata an executable workflow within `providerd`:
versioned protocol profiles and configured seeds, explicit official discovery,
Ed25519-authenticated remote metadata, then private administrator overrides.
Transport authority remains in the deployment. No research database, service,
RPC schema, SDK dependency or vendor branch in the Loop is added.

Publication uses the existing durable exclusive-link writer with bounded,
hash-linked generations. Runtime activation builds all routes before a single
state swap. Retained resolutions preserve exact model and price after reload
and process restart. Availability, source validity and historical invocation
verification are separate. Unavailable discovery remains explicit for cloud,
vendor and gateway routes; no inventory endpoint is guessed.

## Acceptance cases

- Actual HTTP native/list fixtures validate headers, read-only endpoints,
  partial capabilities, deterministic ordering, pagination, cycles, duplicate
  IDs, malformed bodies, byte limits, redacted failures, cancellation and no
  redirect/retry leakage.
- Signed catalogs validate pinned Ed25519 keys, source identity, canonical bytes,
  validity, tamper rejection and credential-free fetches. Strict metadata cannot
  replace endpoints, secrets, plugins or caller authority. Layer precedence and
  persistent source revision watermarks are exercised.
- TLS/gRPC invocation validates immutable price and model pins, successful
  atomic reload, in-flight continuity, corrupted-update rejection, expiry,
  clock regression, sticky retirement and exact successful receipt binding.
- Independent 2/4/8 OS publishers synchronize at a barrier: only one successor
  can publish. Kill/restart interrupts the real fsync/link path immediately
  before publication and after durable completion. Pending files never activate;
  committed bytes cannot be overwritten. Retention capacity fails closed.
- The compiled executable publishes/inspects a catalog, serves authenticated
  invocation against a local compatible upstream, activates `SIGHUP`, denies
  a newly unaffordable pin, retains the old one and restores it after restart.

The complete TypeScript run passed **676 Provider cases across 21 files** in
115.01 seconds, including **82 new catalog/discovery cases**, and **119 protocol
cases**. The Web bootstrap has no behavioral tests and is not UI acceptance.
`just check` passed: all 3,919 Python/Rust/Shell and 666 TypeScript/JavaScript
function declarations comply with naming limits; protocol generation and wire
fixtures, Rust fmt/Clippy (`-D warnings`), TypeScript format/lint/types and all
four Python environment gates passed. Protocol, Provider and Web builds passed.
No quality assertion was disabled.

```bash
./scripts/pnpm.sh --filter @loop-engine/providerd typecheck
CI=true ./scripts/pnpm.sh test
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CI=true just check
CI=true ./scripts/pnpm.sh build
```

No paid generation, production credential or external live model was used.
HTTP/TLS, OS-process and journal behavior is real; suppliers are local fixtures.
Fixture teardown removes project-specific temporary directories. Tests do not
touch production research data or any other project's `/tmp` files.

## Operations and rollback

The [catalog guide](../development/model-catalog.md) specifies source formats,
signing bytes, publish/describe/reload commands, capabilities, verification
limits, retention and authority epochs. Disable refresh writers before rollback;
preserve all immutable catalog and invocation evidence. Restore the previous
binary/configuration or publish reviewed metadata as a new revision. Do not
rewrite records or silently extend a retained pin's validity.
