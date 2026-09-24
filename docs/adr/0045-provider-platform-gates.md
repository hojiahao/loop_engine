# ADR 0045: Provider platform limits and process isolation

Status: implementation, local regression, quality/build and actual-container
gates passed. Task publication and exact-commit CI remain required.
Phase 9 delivery unit 8.

## Requirement and scope

Complete the Provider platform with bounded request admission, exact cost
arithmetic, explicit retry behavior, an actual isolated TypeScript deployment
and a combined protocol acceptance matrix. Existing per-call budgets,
concurrency gates and immutable replay remain in force. This task does not
implement the Phase 10 run lifecycle or claim live supplier entitlement.

## Decisions

Add a small bounded sliding-window admission limiter to the existing Host.
The deployment pins window length, request count, maximum reserved tokens and
maximum reserved USD. Use monotonic elapsed time, integer nanodollar arithmetic
and no timer queue or waiting requests. Deny a full window before creating an
invocation claim or performing outbound work; its key remains retryable after
capacity returns. Count admitted requests conservatively, including cached
replay and attempts which later fail. These are per-process traffic limits,
not a durable account-wide spending ledger. Restart resets the local window;
Phase 10 still owns persistent run budgets and aggregate reservations.

Generation retry remains zero. A complete authenticated replay reads the
immutable result; an ambiguous claim cannot authorize another paid attempt.
Read-only metadata refresh may be retried explicitly within its existing
deadlines. Final usage is priced with the pinned input/cache-read/cache-write
and output rates, rounded up using integers. Additional fees retain their
conservative preflight ceiling; unreported charges do not become invoice facts.

Reuse the pinned DaoCloud Node image for a dedicated Provider runtime with
only compiled Provider/protocol code, Node dependencies and catalog assets.
Mount private Provider configuration, invocation/catalog state and approved
prompt artifacts only. No source repository tree, research/holdout storage,
database credentials, Docker socket or host home is mounted. Drop privileges,
Linux capabilities and write access to the root filesystem; bound memory,
processes and temporary storage. A tested outbound gateway admits only
administrator-selected destinations; the Provider network has no direct
external route. This gateway has infrastructure network authority, not research
or model-routing authority. It does not decrypt supplier TLS or receive keys
in configuration. It is justified by the need for an OS-enforced outbound
boundary, independent of an SDK's request validation.

Run the real compiled Provider with authenticated RPC and local supplier
fixtures inside that namespace. Probe known protected paths, unexpected
environment variables, root writes, direct network escape and denied gateway
destinations. Verify allowed transport still completes a real invocation.
The fixture never receives production credentials or creates a paid request.

## Acceptance and rollback

- Sliding-window request/token/cost limits, exact boundaries, clock regression,
  concurrent callers, replay, rejection before claiming and subsequent recovery.
- Pinned integer pricing goldens for cache reads/writes, reasoning-inclusive
  output, rounding and separately reserved unreported fees.
- Existing complete native/cloud/vendor/compatible/catalog contract matrix;
  no implicit retry/fallback or supplier branches in Loop/research code.
- Actual container filesystem, identity, credentials and network isolation,
  successful approved invocation, denied unapproved destinations and cleanup.
- Product README with verified commands, current interface/data limitations and
  operational guides; keep phase histories in maintenance documentation.
- Full required quality gates, task commit/push and exact-commit CI before
  closing Phase 9 and performing the authorized first merge.

Disable new listeners/writers before rollback. Restore the prior Provider
deployment/binary while preserving all invocation, continuation and catalog
history. Do not erase ambiguous claims or make prior model pins resolve to a
new model. No research schema migration is introduced.
