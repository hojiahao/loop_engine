# ADR 0039: Native content and stream integrity

- Status: Accepted; `daae4d0` pushed, exact-commit CI `35819112668` passed all seven jobs
- Owner: hojiahao
- Extends: ADR 0004, ADR 0019 and ADR 0038

## Requirement

Phase 9 unit 2 makes the existing authenticated ProviderService usable for a
tool-assisted research conversation: native streaming, tool calls and results,
schema-constrained output, reasoning continuation, prompt caching and separately
authorized prompt artifacts. OpenAI Responses/Chat and Anthropic Messages keep
their native semantics. A fragment, successful HTTP status or SDK object alone
is not a completed, validated model response.

## Decision

Keep one request-authorization, budget, token-count and invocation-journal path
for unary and streaming calls. Native adapters translate typed content and own
their protocol-specific stream state. Do not add another service, research table
or Agent-loop vendor branch. The Provider Host returns tool proposals; it never
executes tools. Effective capabilities are the intersection of implemented
transport behavior and the deployment's model declaration.

Use the pinned official SDKs for vendor transport and event decoding. Bound
decoded bytes, event counts, content blocks and time; propagate cancellation and
consumer shutdown upstream. Validate native lifecycle order, indices, terminal
state and final usage. Require complete JSON and local schema validation before
accepting a tool call or structured result. Reject unknown output features rather
than silently dropping them. Stream deltas are previews, not executable results.
Only a validated, durably published response may produce the single successful
terminal event. Missing completion, malformed output and interruption end with
typed non-OK RPC status. Exact completed replay uses the stored result without
another supplier call; an incomplete journal claim remains ambiguous.

Validate JSON using a bounded parser and schema validator; verify the RFC 8785
document/schema fingerprints from the existing contract. Reject duplicate keys,
invalid Unicode, non-finite numbers, unresolved schema references and unsupported
schema features before transport. Do not confuse this JSON profile with the
narrow integer-only canonical profile used for local deployment identities.
Compile only administrator-registered schemas whose exact canonical digest is
pinned in the request policy; caller-supplied schema bytes must match that
registration. This avoids treating arbitrary model/client JSON schemas as
trusted executable validator input. Keep schema size/depth and input bounds even
for registered schemas; registration is not permission to fetch remote references.
Conversation validation binds each tool result to an earlier unanswered call;
tool selection and parallel calls must match the declared capabilities.

Vendor continuation bytes stay in private provider state. Return only a
non-secret record reference bound to the authenticated actor, provider, resolved
model, content digest and expiry. Reauthorize each use, preserve native signed
or encrypted blocks exactly, and reject cross-actor, cross-model, expired or
modified records. Continuation references do not grant data access. Preserve
records required by durable responses across restart and rollback.

Resolve prompt artifacts only from a deployment-owned private prompt namespace,
with actor-scoped access, allowlisted media/schema, bounded size, immutable hash
and no-follow file reads. Never fetch caller URLs or resolve research/holdout
paths. Read and verify bytes once before counting and generation so both see the
same input. Provider namespace checks complement, but do not replace, the actual
process/filesystem isolation gate in unit 8.

Represent cache writes separately from cache reads in additive usage/pricing
fields, without modifying the compatibility baseline. Anthropic input usage is
uncached input plus cache-read and cache-creation input. The preflight reserve
uses the most expensive configured input class and never assumes a cache hit.
Allow only explicitly priced cache retention; leave charged_cost absent without
supplier billing evidence. Reasoning tokens remain a subset of output usage.

## Acceptance

Run each native protocol through the real TLS/gRPC listener and a local HTTP/SSE
fixture. Verify streamed text, multiple tool calls, tool-result continuation,
JSON-schema output, reasoning-state restart and prompt-cache accounting. Include
chunk splits inside UTF-8 and JSON tokens, interleaved content indices, incomplete
JSON, missing/duplicate terminal events, post-terminal output, native error
events, missing/over-budget usage, cancellation and slow consumers. Confirm
private prompt/continuation denials and no raw secrets in RPC failures.

Keep unit 1's authentication, replay, 2/4/8-process and kill/restart tests. Add
schema compatibility and generated-binding checks for additive fields, plus
negative capability and cost tests. Publish implementation, test commands,
observed results and remaining live-verification limits in the unit's evidence.
No paid call or live-verification claim follows from offline fixtures.

## Rollback

Disable the optional Provider listener or deploy the prior build with its own
model pins. Preserve invocation and continuation records. New configuration and
capabilities must fail explicitly on an older build; do not reuse new pins with
an old executable. No SQL migration, research-history rewrite or holdout unlock
is part of this change.

## References

- [OpenAI streaming](https://developers.openai.com/api/docs/guides/streaming-responses)
- [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling)
- [OpenAI structured output](https://developers.openai.com/api/docs/guides/structured-outputs)
- [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)
- [Anthropic streaming](https://platform.claude.com/docs/en/build-with-claude/streaming)
- [Anthropic structured output](https://platform.claude.com/docs/en/build-with-claude/structured-outputs)
- [Anthropic prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
