# ADR 0019: Runtime identity and data authority

- Status: In progress
- Date: 2026-09-11
- Owner: hojiahao

Implementation and local `just check/test/build/doctor/test-isolation` gates
pass. Commit, push and remote acceptance remain pending at this pre-publication
record. Detailed evidence: `docs/verification/phase-04-research-integrity.md`.

## Requirement

Phase 4 unit 4 connects verified connection identities to durable commands and
actual artifact access. An Actor, URI, grant ID or successful historical receipt
is not authority. Discovery and provider processes must be unable to read the
protected store even when given a correct file name. This task does not implement
factor computation, licensed market-data quality or a production holdout run.

## Decision

Reuse tonic's mutually authenticated TLS transport. A deployment-owned registry
pins leaf-certificate SHA-256 digests to actors and exactly one runtime role.
The TLS stack validates the client chain and proof of private-key possession;
the registry supplies application identity. Forwarded identity headers and
caller Actor values never authenticate a connection. Unknown, expired, duplicate
or revoked identities fail closed. Registry reload is an explicit restart, so a
run never silently changes its authority policy.

Register the existing JobService behind this boundary. Enabling the listener
requires explicit private deployment configuration; the existing health/readiness
HTTP listener does not gain mutating routes. Only deployment-pinned jobs may be
read or mutated, and ordinary research workers cannot act on protected jobs.
Store authorization, immutable receipts, revisions and lease fencing still apply.

Protected data additionally requires a short-lived opaque capability in protected
gRPC metadata, bound to the authenticated subject, exact job and lease, audience,
and expiry. Capabilities are not serialized into jobs, receipts, logs or model
messages. A restart invalidates transient capabilities without reopening a grant.
Issuance follows a currently valid consumed-plan job/lease, never a historical
acquisition response alone. Every use rechecks durable state. Discovery, provider
and ordinary development identities can neither receive nor present one.

Keep prompt, development, protected source and job-view stores separate. The
broker resolves only server-pinned references, checks actual bytes and publishes
read-only views containing only the authorized artifacts. Workers mount only
their declared view; discovery and provider never mount the protected source or
hold its storage/database credentials. File checks reuse ADR 0018's no-follow
content-addressed reader. Large data stays outside gRPC. Publication acceptance
and filesystem delivery are distinct; retry must reauthorize rather than treating
a receipt as permission. Previously disclosed bytes cannot be made unknown again.

Only synthetic/public-development paths are enabled before Phase 5 data-quality
and Phase 7 protected-backtest semantics are available. Protected tests use real
files, TLS connections and PostgreSQL state with explicitly synthetic plans;
they do not establish market-data quality or enable real holdout evaluation.

## Gates And Recovery

Verify missing/untrusted client certificates, forged actors/metadata, role and
job confusion, capability disclosure/replay/expiry, lease loss, clock regression,
revocation, changed files, cancellation and restart. Exercise real OS processes
and read-only namespace denials, including a known protected path. Keep all
existing 2/4/8-writer and commit-boundary tests. Secrets must not appear in error
messages, audit receipts or exported artifacts.

Disable the optional runtime listener and view publisher to roll back. Preserve
jobs, trials, grants, receipts, immutable objects and audit history. Do not reset
periods or delete schemas. No production migration, approval, grant, paid data
request or LLM call is authorized by these local acceptance tests.
