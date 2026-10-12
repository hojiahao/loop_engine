# Phase 10 unit 1: Authenticated invocation recovery

Status: implementation, complete local regression, quality/build and actual
container gates passed on 2026-09-28. Commit `ac2d730` is pushed on
`feature/run-harness`; exact-commit CI `36390979127` passes all seven jobs,
including Rust, unified workspace and the clean DaoCloud container. Unit complete.

## Requirement and delivered behavior

ADR 0046 adds an executable read-only recovery RPC to the isolated Provider.
Fresh transport-authenticated queries recover old immutable responses without
mutating the original command, sending another paid request or requiring the
old model route. `--recover-only` is the installed process workflow when current
generation catalog activation cannot succeed. It accepts no new generation and
requires existing journal storage; it does not invent a new empty history.

Identity binds the authenticated actor, original key, original request ID and
original command digest. Ambiguous and absent evidence do not authorize resend.
Filesystem reads retain and revalidate the journal directory identity, use bounded
buffers and reject unsafe modes, symlinks, corrupt checksums and malformed nested
response content. Concurrent publication of a partial claim remains ambiguous.
Historical reservations remain upper bounds, not measured supplier charges.

## Observed acceptance

- Shared Rust, TypeScript and Python lookup request/result fixtures pass:
  three Rust tests, eight TypeScript targeted tests and eight Python targeted
  tests. Baseline/history inventory, additive descriptor, service import boundary,
  method availability and regenerated wire fixture checks pass. Baseline bytes
  remain unchanged; no metadata feature negotiation is falsely claimed.
- Initial complete TypeScript regression passed 722 Provider and 122 protocol
  tests before the recovery-only and filesystem-review additions.
- Updated lookup and compiled CLI suites pass 49 tests: actual mTLS/gRPC,
  completed/absent/ambiguous evidence, removed model/secret, old invocation time,
  exhausted generation allowance, actor/certificate/metadata denial, fresh query
  IDs, cancellation, malformed evidence and valid rich-content recovery.
- The CLI cases run actual independently spawned compiled services against missing
  and expired catalogs with no supplier secrets, recover the original response,
  reject both InvokeModel and StreamModel, and shut down cleanly. Missing journals
  and contradictory command modes are denied without writes.
- Fourteen filesystem cases pass with real files plus deterministic interleaving
  hooks: claim growth, result growth/size bounds, directory removal/replacement/
  symlink, retained directory handles and non-creating read-only startup.
- Four lifecycle cases pass for clock regression, shared read/generation capacity,
  and cancellation/deadline retention until the underlying OS read settles.
- `just check` passes, including Rust fmt and full all-targets/all-features Clippy
  with `-D warnings`, protocol regeneration/boundaries, TypeScript format/lint/types
  and Python quality checks. Naming gates accept 3,929 Python/Rust/Shell and 723
  TypeScript/JavaScript declarations.
- Final serial TypeScript regression passes **753 Provider tests across 26 files
  and 122 protocol tests**; workspace TypeScript build passes. This includes the
  existing CLI/catalog suites that timed out during the earlier overlapping build.
- The final actual-container gate passes in **19.76 seconds**: the unprivileged
  compiled Provider completes a synthetic model invocation and returns that exact
  saved response through LookupInvocation, inside the existing data/egress boundary.

The first review caught the difference between constructing a Host in a test and
starting its real executable: normal catalog activation could prevent recovery,
and normal bootstrap could create a missing journal. The explicit recovery-only
mode and independent CLI tests address both, while normal startup stays strict.
No production key, licensed research data or paid model was used.

During the first full-quality run, the new lifecycle test omitted generated
message type markers; TypeScript caught this, and the corrected `just check`
passed. An overlapping cold Clippy build coincided with container and old CLI
startup timeouts. Host sampling recorded swap I/O and 25-46% I/O wait. Those
attempts are failed evidence, not passes; final regression and container tests
were rerun serially without changing their original deadlines. The independent
container client's new query initially also omitted generated nested-message
type markers; fixing the fixture allowed the unchanged service to pass its actual
container gate. No failure was suppressed and no deadline was extended.

## Reproduction

```bash
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CI=true just check
CI=true ./scripts/pnpm.sh test
CI=true ./scripts/pnpm.sh build
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 ./scripts/cargo.sh test --locked --offline --workspace --all-features --test provider_lookup
./scripts/uv-protocol.sh run --locked --offline pytest tests/test_provider_lookup.py tests/test_wire_compatibility.py
node --test --test-isolation=none tests/runtime/provider.test.mjs
```

The container client invokes and recovers the exact response through real mTLS
inside the existing read-only, unprivileged namespace. This also verifies Linux
directory-descriptor access under the deployed isolation policy. Tests clean only
their own uniquely named temporary directories, processes and container resources.
CI now includes `feature/**` push events for the new post-merge delivery branch.

## Limits and rollback

This unit does not implement durable Rust budgets/model dispatch, autonomous
Loop execution, research admission or live supplier entitlement. It provides the
recovery operation those workflows need. A receipt is not proof of statistical or
economic factor quality. RPC availability is the delivered capability gate;
UNIMPLEMENTED is not ABSENT and must never fall back to generation.

Stop recovery callers before restoring the previous Provider binary. Preserve
all journal files and any caller-side budget reservations; no database or artifact
migration is required. Follow [the recovery guide](../development/invocation-recovery.md).
