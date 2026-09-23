# ADR 0038: Authenticated native model invocation

- Status: Accepted; fc00680 pushed, CI 35701264148 passed all seven jobs
- Owner: hojiahao
- Extends: ADR 0001, ADR 0004 and ADR 0019

## Requirement

`providerd` currently exposes only health. Deliver a callable instance of the
existing ProviderService using native OpenAI Responses/Chat and Anthropic
Messages. Keep provider code out of the Rust control plane and Python research
worker. Do not mislabel local contract tests as successful paid API access.

## Decision

Use the official TypeScript SDKs behind small vendor-owned plugins, with SDK
automatic retries disabled. Reuse generated Protobuf descriptors with the
Connect Node adapter for HTTP/2 gRPC rather than hand-writing gRPC framing.
An optional, explicitly configured mTLS listener is separate from health.
Pin client certificate fingerprints to actors; caller Actor fields and forwarded
headers do not authenticate. Server configuration pins model resolution, request
policy, credential environment references and budgets. Requests cannot provide
an endpoint or change provider capabilities/pricing.

The plugin fingerprint commits to installed host sources, package metadata and
the workspace dependency lock. A compiled deployment and its `--describe` output
must come from the same build. Domain-separated canonical JSON pins the local
model/catalog/policy profile; it does not change canonical research identities.

This first delivery supports bounded unary text conversations and explicit
refusals. Rich content, tools, structured output, reasoning continuation and
streaming remain unit 2 and are denied before transport in unit 1. Effective
capabilities must not advertise those unfinished paths. Provider responses are
validated before producing a successful domain response; unknown/incomplete
output, missing usage and operational failures fail closed.

Check native input-token counts before generation; bound output tokens, request
and response bytes, concurrency and total operation time. Reserve the full
configured input/output allowance against the per-call cost budget using exact
decimal arithmetic and pinned prices. Counts and price-derived costs are not
supplier invoices: leave `charged_cost` absent without actual billing evidence.
Provider count estimates can differ from final usage; reject over-budget final
usage and preserve uncertainty rather than claim remote billing can be undone.

Use a private, provider-only invocation journal for duplicate-spend protection.
Claim an authenticated idempotency key exclusively before outbound work; store
only a request digest in that claim and publish the bounded result atomically.
An interrupted claim without a completed result is ambiguous and cannot be
automatically resent, including after restart. This journal is separate from
research state and does not replace Phase 10's durable run-wide budgets. Keep
secrets out of receipts/errors and never mount research or holdout storage.

## Acceptance and recovery

Exercise real TLS/gRPC against local HTTP vendor fixtures, exact native request
shapes and usage normalization, certificate/actor confusion, policy/model drift,
unsupported content, budget/deadline/cancellation, redirects, oversized or invalid
responses, SDK error redaction and duplicate/concurrent/crash invocation paths.
Retain descriptor/dependency boundaries and concise function naming.

Disable the optional Provider listener to roll back; the health endpoint retains
its bootstrap behavior. Preserve private invocation claims/results so uncertain
calls cannot be silently retried. No SQL migration or research-history rewrite
is needed. Publish test evidence and the task's Chinese commit before continuing.

## References

- [OpenAI TypeScript SDK](https://github.com/openai/openai-node)
- [OpenAI token counting](https://developers.openai.com/api/docs/guides/token-counting)
- [Anthropic TypeScript SDK](https://github.com/anthropics/anthropic-sdk-typescript)
- [Anthropic token counting](https://platform.claude.com/docs/en/build-with-claude/token-counting)
- [Connect Node server adapter](https://connectrpc.com/docs/node/server-plugins/)
