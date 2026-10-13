# Phase 11 unit 3a: Verified feedback and leased perturbation

Status: `in_progress`, not accepted, merged, released or deployed. This record
describes the implementation and required evidence on `feature/research-feedback`.
It must be updated with actual final command results and CI links before this
delivery is marked complete. Phase 11.2's acceptance does not cover these changes.

## Requirement and design

An explicitly selected v3 research plan alternates fixed Discovery and numerical
Perturbation proposal stages. Each proposal uses the existing evaluation and
portfolio workflow. Earlier registered IS results supply bounded, structured
feedback; eligible finite Sharpe observations also inform the existing installed
window perturbator. Full model requests, source identities, family and random
state are immutable between committed transitions. No model receipt is invented
for numerical work.

The design is [ADR 0054](../adr/0054-adaptive-research-workflow.md); executable
administrator and operator instructions are in the
[research run guide](../development/research-runs-cli.md#adaptive-workflow-opt-in).

Perturbation is a real internal job, with a Research service identity, one
reserved step, zero model tokens/USD, a bounded worker lifetime and an existing
job lease. Generic submission/completion cannot bypass its private source and
completion proofs. Its immutable work and successful receipt/step live in the
development evidence store. Selection itself creates no factor trial; actual
evaluation submission does. Numerical work executes outside database
transactions. The parent commits state, reservations, next child/trial, immutable
receipt and audit together after checking the completed source.

Reuse the existing run binding, job queue, worker, Provider receipts, file guards
and PostgreSQL transactions. Migration 0017 is additive and fences incompatible
writers. Public versions 0 and 2 retain their old field restrictions and behavior.
The v3 output exposes safe digests and counts, not raw messages, work, seeds,
leases or protected capabilities.

## Required acceptance

| Boundary | Acceptance requirement | Current evidence |
| --- | --- | --- |
| Three-language contracts | Old request/response compatibility; v3 fields/feature bounds; genuine Perturbation shape and zero-model budget; no public arbitrary-work route | Shared fixture and Rust/TS/Python cases implemented; targeted TS and complete Python protocol checks passed; Rust execution pending |
| Private context | Exact installed template plus server-verified feedback; bounded messages; replay matches the complete stored invocation; static executors reject dynamic history | Context guards and focused tests implemented; Rust execution pending |
| Public execution fence | Generic Discovery Execute/Resume/Reconcile cannot replace a queued v3 child's frozen invocation; safe status/stop remain available | Installed `generic_bypass` with a controlled-tool plan implemented; not yet passed |
| Real three-round flow | Discovery → evaluation → portfolio; leased Perturbation → evaluation → portfolio; next Discovery receives both verified earlier results | Installed `adaptive_roundtrip` implemented; not yet passed |
| Source semantics | Earlier same-run registered IS result and actual bytes; correct Sharpe profile; unavailable statistic never becomes zero; no protected evidence | Manifest/proof tests implemented; final execution pending |
| Random state/recovery | Restart before selection and after parent commit; exact receipt replay; one applied transition and no additional worker/model call | Roundtrip and independent-process commit/crash cases implemented; not yet passed |
| Independent writers | 2/4/8 independent installed CLI processes race one parent revision; one actual worker launch, one next child, one state advance | `adaptive_writers` implemented; not yet passed |
| Failure integrity | Duplicate/failed/coverage paths create no fabricated trial or score; infrastructure failure remains distinct; unavailable observation uses a frozen model fallback | Required targeted coverage and final execution pending |
| Corruption | Changing actual frozen feedback or a consumed historical selection receipt prevents the next stage, leaves queued state and accounting unchanged | Installed `corrupt_feedback` and `selection_history_corruption` implemented; not yet passed |
| Worker lifecycle | Lease fencing, cancellation, expiry and crash/restart before/after child completion; no unleased or repeated numerical execution | Storage/proof cases and before/after-commit process matrix implemented; full execution and cancellation gate pending |
| Budget and termination | Frozen parent and child ceilings, no refunds, deadline checks, actual-round `search_exhausted` without fake evaluation | Contract cases implemented; end-to-end terminal/budget acceptance pending |
| Regression/release | Default thread stack, numerical goldens, formatter, Clippy, workspace/container gates, Chinese task commit, push and remote checks | Pending |

The installed fixtures use 18 synthetic sessions and six securities to produce
enough daily returns for the frozen statistical minimum. A loopback supplier
returns the explicit first `ma(market.close,5,2)` AST and a later distinct model
candidate. Provider transport, installed Python calculations and PostgreSQL are
real. A private test-only executable shim records actual perturbation launches,
then immediately executes the frozen interpreter; it does not manufacture work,
outputs or statistics. No production worker hook is added.

Expected complete-round accounting is nine children, two model steps, six
numerical trials, three evaluations and three registered portfolios. The
perturbation child has zero model steps. These are executable test assertions,
not reported pass results. The 2/4/8 test races the second-round perturbation
against parent revision 4 and expects one revision-5 evaluation reservation,
five total jobs and three actual numerical trials.

The commit/crash cases launch a separate library-test runtime process loading the
same private deployment, real database and installed workers, because production
binaries contain no fault hooks. They reuse existing test-only fault points at
child completion before/after commit and parent advancement before/after commit.
After an uncommitted child result is lost, the original lease/deadline is allowed
to expire; recovery must record a budget failure without another calculation or
fabricated successful selection. After committed child success, recovery reuses
that receipt and applies exactly one parent random-state transition.

## Reproduction

Use a bootstrapped checkout and the disposable local PostgreSQL fixture only.
Fresh installed-process tests need binaries and a freshly compiled Provider
host from this exact source tree. Do not run against production, change thread
stack settings to obtain a pass, or run competing Cargo builds on a small host.

```bash
export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
./scripts/pnpm.sh --filter @loop-engine/providerd build
./scripts/cargo.sh build --locked --offline --workspace --all-features --bins

./scripts/cargo.sh test --locked --offline -p loop-protocol --all-features \
  --test adaptive_boundary --test run_boundary
./scripts/pnpm.sh --filter @loop-engine/protocol exec vitest run \
  test/adaptive.test.ts test/run-boundary.test.ts test/job.test.ts
./scripts/uv-protocol.sh run --locked --offline --no-sync pytest

bash scripts/postgres-test.sh start
./scripts/cargo.sh test --locked --offline -p loopd --all-features --lib -- \
  manifests::feedback manifests::perturbation \
  store::perturbation_job runtime::discovery::context \
  runtime::discovery::tests::adaptive --test-threads=1
./scripts/cargo.sh test --locked --offline -p loopctl
bash scripts/postgres-test.sh usage
bash scripts/postgres-test.sh stop

just check
just test
just build
just doctor
```

Stop the disposable database even if a focused test fails. The managed
`just test` runner handles its own database lifecycle; do not overlap it with an
independently managed suite. Fixtures own their temporary directories and must
clean them after their processes exit. Preserve committed research evidence,
source objects, receipts, database history and unrelated project files.

## Observed evidence

- Targeted TypeScript adaptive/run/job contracts passed 87 cases. Type checking
  passed. The complete Python protocol suite, mypy and Ruff passed at the
  contract integration checkpoint.
- Descriptor privilege-boundary checks passed with the new internal task
  excluded from public Discovery/Run request reachability.
- The first focused Rust run passed 36 feedback, source-semantics, immutable-work,
  private-context, submission/lease and transition tests against the disposable
  PostgreSQL fixture. Nineteen Python portfolio-statistics golden tests passed.
  These do not substitute for the installed-process acceptance below.
- Discovery context, CLI and installed adaptive fixtures passed Rustfmt parsing
  and formatting. The loopback supplier script passed Node syntax checking and
  Biome. These checks do not establish Rust compilation or execution.
- Implementation commit `7a1d7ba` was pushed in draft
  [PR #5](https://github.com/hojiahao/loop_engine/pull/5). Initial branch and PR
  checks found a large command enum in strict Clippy; both payloads now use
  indirection, retaining the check rather than suppressing it. Final compilation,
  installed behavior, concurrency/crash acceptance and green remote verification
  remain pending in this record.
- No live model benefit, paid dataset coverage, improved research returns,
  production migration or formal factor admission is claimed.

## Rollback and preservation

Disable new v3 writers by removing only v3 wrappers from the deployment's
`runs.plans`, retaining v1/v2 plans and restarting a migration-aware compatible
runtime. If no plans remain, omit `runs` rather than leaving an invalid empty
catalog. Keep owner-scoped status readers and immutable research/audit history.

Do not downgrade to a writer that cannot honor migration 0017, reset random
state, refund ambiguous model charges, recreate missing receipts, or delete the
database/derived evidence as a rollback. Restoring the exact registered plan and
compatible writer re-enables its original state; it does not authorize replacing
its policy or regenerating an uncertain model invocation.
