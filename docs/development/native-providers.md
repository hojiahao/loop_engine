# Native Provider conversations

The TypeScript `providerd` has an optional TLS 1.3 / HTTP/2 gRPC listener. It
implements `loop.provider.v1.ProviderService/InvokeModel` and `StreamModel` for
six direct native paths: `openai_responses`, `openai_chat`, `anthropic`,
`google_generate`, `google_interactions`, and `cohere`, plus cloud deployment
routes `azure_responses`, `azure_chat`, `vertex_generate` and `bedrock_converse`.
See [cloud Provider configuration](cloud-providers.md) for cloud identity,
deployment mapping, native capability limits and rollback.
The bootstrap HTTP health endpoint remains at `127.0.0.1:8090` and cannot invoke
a model. No listener is enabled by an API key alone.

Conversations support text, function-tool proposals/results, registered JSON-schema
output, images/PDFs from private prompt artifacts and ordered streaming. Responses
and Anthropic support private reasoning continuation; Google and Cohere use
their own native continuation formats described below. All system messages
precede conversational messages; a request ends with a user or tool-result turn.
Optional capabilities default to **disabled** and must be verified for the exact
configured model before enabling. A plugin's implementation does not prove that
every model implements its features. Other suppliers, model discovery and catalog
reload remain later Phase 9 units; run-wide scheduling/budgets remain Phase 10/11.
This is not a completed autonomous research loop.

## Private deployment

Use Linux with the locked Node 24.17.0 / pnpm 11.25.0 toolchain:

```bash
./scripts/pnpm.sh --filter @loop-engine/protocol build
./scripts/pnpm.sh --filter @loop-engine/providerd build
```

Copy `config/providers/native.example.json` to a private path outside Git and
replace every `REPLACE_*` value. The example deliberately fails validation until
configured. Obtain a server certificate (DNS SAN `localhost`) and a client
certificate from your own private CA; use separate private keys. Set private
configuration, CA certificate, server certificate and key files to mode `0600`,
owned by the service identity. Use an absolute provider-only journal directory
with mode `0700` under an existing administrative parent. Startup creates only
the final directory and fsyncs it and its parent before accepting work. Do not
put the journal in `/tmp`, a research artifact store or
a shared writable directory; do not mount research/holdout data or give this
process database credentials. The root filesystem/egress deployment isolation
acceptance remains Phase 9 unit 8; mTLS and rejected artifact messages alone do
not establish OS isolation.

Compute each approved client's certificate fingerprint over DER bytes:

```bash
openssl x509 -in /private/loop-provider/client.pem -outform DER | sha256sum
```

Register that lowercase hex value under `principals`, with its exact actor ID,
actor kind and allowed model configuration IDs. A certificate signed by the CA
but absent from this registry is denied. Forwarded identity, authorization and
holdout/capability metadata are denied. Configuration changes require a restart;
they cannot mutate an already resolved request silently.

Resolution pins bind the installed host files, package metadata and workspace
lockfile. Build and describe from the same checkout: source-test pins and compiled
deployment pins intentionally differ. Dependency or executable changes require
a new resolution; never reuse a saved pin with a different build.

For each model, verify the exact versioned model ID, its account availability,
context/output limits and current USD-per-million token prices. The returned
native model ID must equal the configured ID when the protocol returns it;
mutable supplier aliases that resolve differently fail closed. Cohere V2 does
not echo the resolved model ID, so it attests only the exact requested ID, not
independently proven supplier revision. Use a versioned Cohere ID, not a mutable
alias. `input_usd`, `output_usd` and `cached_usd` are
normalized non-negative decimal strings with at most nine fractional digits.
`secret_env` names an environment variable beginning `LOOP_LLM_`; it never
contains a secret value. Inject the keys through a protected service environment
or secret manager, not a command-line argument, Git, log or chat. The example
uses `LOOP_LLM_OPENAI_API_KEY` and `LOOP_LLM_ANTHROPIC_API_KEY`.

Inspect the exact model/policy pins without a model call:

```bash
PROVIDERD_DEPLOYMENT=/private/loop-provider/deployment.json \
  node apps/providerd/dist/index.js --describe
```

Start the optional listener (default example: `127.0.0.1:8091`):

```bash
PROVIDERD_DEPLOYMENT=/private/loop-provider/deployment.json \
  node apps/providerd/dist/index.js
```

The process does not call a supplier at startup. Invalid private configuration
fails with a redacted error. Health readiness does not attest credentials, model
availability or paid live verification. Keep API output and deployment snapshots
private. This delivery has offline contract evidence, not live verification.

## Calling the existing protocol

Use the generated `@loop-engine/protocol/provider` client descriptors with an
HTTP/2 gRPC client, the private CA, approved client certificate and private key.
For example, the TypeScript transport is `createGrpcTransport` from
`@connectrpc/connect-node`, with `nodeOptions: {ca, cert, key}`. The service is
`ProviderService` and the unary client method is `invokeModel`.

An `InvokeModelRequest` must contain:

- A `CommandContext` with a fresh request ID, correlation ID, idempotency key,
  registered actor/kind, and a timestamp no more than five minutes from server
  time. Transport identity must match this descriptive actor.
- A `ModelInvocation` with the same request ID and an **unchanged** model
  snapshot and `request_policy` from the running build's `--describe` output.
  Use Protobuf JSON parsing (`fromJson`) to recover typed snapshots; no digest
  or capability field supplied by the client becomes server authority.
- Bounded text messages and explicit positive input/output token limits, USD
  budget and wall-time duration. The gRPC call also requires a timeout no greater
  than the deployment's `wall_time_ms`.

No endpoint URL is accepted in a request or this deployment schema. Native SDK
endpoints are fixed to the official supplier origins, redirects are refused,
SDK automatic retries/logging are disabled, and upstream decoded response bytes
are bounded. OpenAI uses Responses input-token counting for both paths; a Chat
model for which that counter is unavailable is denied rather than given a guessed
count. Anthropic uses its native Messages counter. Google GenerateContent sends
the documented `generateContentRequest` envelope to countTokens, including system
instructions and tool declarations. Its SDK's Developer API counter omits these
fields, so this one operation uses bounded native HTTP directly. Counters may differ from final
usage; an observed overrun is an explicit failure, not a reversible supplier
charge. Maximum output tokens are also sent to the supplier.

Cohere V2, Google Interactions and cloud routes use a conservative reservation
in this profile. Their deployment entries **must** specify `input_token_limit`, the
vendor-documented maximum input for the exact model. Reserve that entire ceiling:
`budget.maximum_input_tokens` and `policy.input_tokens` must cover it, and the
USD budget must cover the full input/output reservation. Do not set this value
to a guessed count or a smaller desired budget. The ceiling cannot exceed
`context_tokens`. The context value is the verified combined input/output
capacity; for a model with separately documented input/output capacities, use
their verified combined capacity. Unlike a measured count, an upper bound plus
maximum output need not fit simultaneously. The vendor can reject an oversized
actual request; final input/output usage must fit the pinned context and both
caller limits. No raw-text tokenize call is represented as an exact chat count.

The preflight reserve uses the full input/output allowance and pinned prices
with integer nanodollar arithmetic, rounded up. The input price used is
the maximum of uncached, cache-read and configured cache-creation prices. No
cache hit is assumed. Reported cached tokens are included in total input tokens;
Anthropic total input is uncached plus cache-read plus cache-creation tokens.
Cache writes have their own `cache_creation_input_tokens` usage field.
`ModelUsage.charged_cost` remains absent: token-price calculations are not invoices.
These are per-call limits; do not treat them as a durable cumulative run budget.

## Enabling model features

Add the following object inside a model entry only for capabilities confirmed
for that model. The example enables streaming and functions but keeps other
optional features disabled:

```json
"features": {
  "streaming": true,
  "tools": true,
  "parallel_tools": true,
  "structured_output": false,
  "vision": false,
  "documents": false,
  "prompt_caching": false
}
```

`parallel_tools` requires `tools`; `documents` requires `vision`. Configuration
changes require a restart and fresh `--describe` pins. Do not invent a capability
by modifying a returned snapshot. The implemented native differences are:

| Feature | Responses | Chat Completions | Anthropic Messages |
| --- | --- | --- | --- |
| Text, function tools, streaming | Supported | Supported | Supported |
| Registered schema output | Native text format | Native response format | Native output config; strict required |
| Prompt images/PDFs | Inline verified bytes | Inline verified bytes | Inline verified bytes; automatic image detail only |
| Reasoning continuation | Encrypted native item | Unavailable | Signed/redacted thinking blocks |
| Requested prompt cache retention | `in_memory` | Supplier automatic caching | Explicit 5-minute cache; separately priced writes |

For Responses set `reasoning` to `low`, `medium` or `high` only when the exact model
supports that effort and summary/continuation format. For Anthropic use `adaptive`,
or `enabled` together with `thinking_tokens` (at least 1024, strictly below both
configured and per-call maximum output tokens). Forced/named tool choice with
Anthropic thinking is rejected before transport. Chat requires `reasoning: "off"`.

### Google and Cohere routes

Add the required model entries to the same private deployment and grant their
`id` values through the existing principal `model_ids` list. Use these plugin/key
names; keys are secret references, not literal values:

| Plugin | Official origin | Suggested secret reference | Extra configuration |
| --- | --- | --- | --- |
| `google_generate` | `https://generativelanguage.googleapis.com` | `LOOP_LLM_GOOGLE_API_KEY` | Bare versioned model ID, without `models/` prefix |
| `google_interactions` | `https://generativelanguage.googleapis.com` | `LOOP_LLM_GOOGLE_API_KEY` | Bare versioned ID plus verified `input_token_limit` |
| `cohere` | `https://api.cohere.com` | `LOOP_LLM_COHERE_API_KEY` | Versioned model ID plus verified `input_token_limit` |

For example, this deliberately incomplete model entry must be filled with
verified numeric limits and prices before it will validate:

```json
{
  "id": "google-checker",
  "plugin": "google_interactions",
  "model": "REPLACE_WITH_VERSIONED_MODEL_ID",
  "alias": "research-google-checker",
  "context_tokens": "REPLACE_WITH_INTEGER_COMBINED_CAPACITY",
  "input_token_limit": "REPLACE_WITH_INTEGER_MAXIMUM_INPUT",
  "output_tokens": "REPLACE_WITH_INTEGER_MAXIMUM_OUTPUT",
  "input_usd": "REPLACE_WITH_VERIFIED_USD_PER_MILLION",
  "output_usd": "REPLACE_WITH_VERIFIED_USD_PER_MILLION",
  "cached_usd": "REPLACE_WITH_VERIFIED_USD_PER_MILLION",
  "features": {"streaming": true, "tools": true, "parallel_tools": true},
  "reasoning": "low",
  "secret_env": "LOOP_LLM_GOOGLE_API_KEY"
}
```

For GenerateContent change `plugin` and remove `input_token_limit`; that route
uses the complete native counter. For Cohere change the plugin/key and use
`reasoning: "off"` for non-thinking models, or `enabled` with `thinking_tokens`
for models supporting native thinking. Thinking budgets must be at least 1024
and smaller than configured and requested output limits. Changing these fields
changes the resolution and capability fingerprints. Google effort settings are
`low`, `medium` or `high`, enabled only when the exact model supports them.
Thinking models must use a thinking-enabled route even for signature-only turns.

| Feature | Google GenerateContent | Google Interactions | Cohere V2 Chat |
| --- | --- | --- | --- |
| Text, functions, ordered streams | Supported | Supported | Supported |
| Registered schema output | `responseJsonSchema` | Native text response format with schema | `json_object` and `json_schema`; incompatible with tools |
| Image/PDF prompt artifacts | Inline bytes; automatic image detail | Inline bytes; automatic image detail | Image bytes; PDFs denied |
| Thinking | Signed native Parts, including function calls | Separate signed thought steps | Native thinking content; private continuation reference |
| Tool selection | Auto/none/required/named | Auto/none/required/named | Auto/none/required; named selection denied |
| Strict functions | Native validated/required mode plus local schema checking | Native validated/required mode plus local checking | One `strict_tools` setting; mixed strict flags denied |
| Cache usage | Reported implicit cache reads | Reported implicit cache reads | Explicit `usage.cached_tokens` only |

All routes return client-executed function proposals; no hosted search, code
execution, grounding, audio, video or generated media is enabled. Interactions
uses the current step-based v1beta API with `store: false` and `background: false`,
never `previous_interaction_id`. This disables the API's stored conversation
feature, not the supplier's separate data-retention policy. Request/response
schemas differ from the older output-based Interactions API.

Google thinking token counts are added to output and reported as its measured
subset; native prompt-tool tokens, if reported, are added to input. Complete
native totals must reconcile. Cohere uses `usage.tokens`; `billed_units` is not
interchangeable with actual tokens and does not prove cache hits. Cohere does not
separate measured thinking tokens here, so that subset remains zero rather than
being invented. Its tool plan is replayable assistant text. Neither route writes
an explicit cache resource or claims cache-creation usage or an invoiced cost.

Preserve all returned assistant content when continuing a Google thinking turn.
GenerateContent stores the full signed Parts privately and checks their visible
text/tool projections before replay; changing a tool invalidates that continuation.
Legacy Gemini function calls without IDs receive stable local correlation IDs;
these local IDs are not invented as native wire IDs. Interactions retains the
separate thought signature. Missing signatures, unknown steps, malformed streams,
model drift or usage mismatch fail closed. The host supports text thought summaries,
not image thought summaries. Offline fixture success is `contract_verified`
evidence only; no supplier credentials were used for this delivery.

Enabling Anthropic `prompt_caching` requires a separately verified
`cache_creation_usd` price. This deployment profile requests only 5-minute
retention; an observed one-hour cache write is rejected. Other plugins reject
`cache_creation_usd`. The OpenAI transports still account for reported cache hits
when no cache preference was requested. No cache hit or exact cost reduction is
guaranteed by a capability flag.

## Registered schemas and tools

Only an administrator-registered JSON schema may enter the model request. Create
a private file containing its exact RFC 8785 canonical UTF-8 JSON, without BOM or
a trailing newline. Set mode `0600`, compute its SHA-256, and register the immutable
file in the deployment:

```json
"schemas": [{
  "id": "factor-window",
  "version": 1,
  "sha256": "REPLACE_WITH_CANONICAL_SCHEMA_SHA256",
  "path": "/private/loop-provider/schemas/factor-window.json"
}]
```

For example, the canonical bytes for a bounded integer window schema are:

```json
{"additionalProperties":false,"properties":{"window":{"minimum":1,"type":"integer"}},"required":["window"],"type":"object"}
```

Compute `sha256sum /private/loop-provider/schemas/factor-window.json`; its result
must match both registration and `JsonSchema.schema_sha256`. The request also
includes the same ID, version and canonical bytes. A changed, missing or unregistered
schema fails before any supplier request. Registration is part of the policy pin.
Schemas use the supported strict Ajv JSON Schema profile: no regex `pattern`,
`patternProperties`, `format`, remote references, asynchronous validators or data
extensions. Schemas are limited to 64 KiB; documents are limited to 256 KiB, depth
32 and 16,384 parsed nodes. Duplicate keys, non-finite numbers and malformed
Unicode are rejected; validation never coerces, inserts or removes fields.

Set `ToolDefinition.strict` explicitly (including when choosing `false`) and use
an object-root input schema. `ToolChoice` can be auto, none, required or a registered
name. The host returns proposals and never executes them. After a completed
`TOOL_CALL` result, append its assistant content, then exactly one `ToolResultContent`
for each unanswered call under role `TOOL`, with explicit `SUCCESS` or `ERROR`.
Each result must reference that call ID. Unmatched, duplicate and unresolved calls
fail validation. Tool results may be text, registered JSON documents, or private
text/JSON artifact references. Error results retain their error status on the wire.

For final schema output, set `structured_output` and an explicit strict choice.
The host validates completed JSON against the registered schema and returns a
`StructuredOutputContent` containing the document's canonical digest. A length
termination is explicitly `LENGTH`, with partial text retained; it is not validated
schema success. Tool arguments are never accepted as an incomplete JSON fragment.

## Prompt artifacts and reasoning state

Optionally configure `prompts` as a private absolute directory separate from
research and holdout storage. A trusted publisher supplies immutable files at:

```text
<prompts>/<SHA-256 of the authenticated actor ID's UTF-8 bytes>/<file SHA-256>
```

The root and actor directory must be owned by the service identity with mode
`0700`; files require `0600`. Symlinks are denied at these checked paths. Publish
with exclusive creation and filesystem durability before referencing the file;
do not change bytes at an existing digest. The host is a read-only consumer, not
an arbitrary URL fetcher or research-data exporter.

`ArtifactRef` uses ID `<file SHA-256>`, URI `loop-prompt://sha256/<file SHA-256>`, raw
SHA-256 bytes, exact byte size, media type and a non-future `created_at`. It has no
row count or manifest. Its schema is `loop.prompt-artifact`, version 1, with the
SHA-256 of RFC 8785 canonical `{"schema":"loop.prompt-artifact/v1","media_type":"<MIME>"}`.
Allowed media are `text/plain`, `application/json`, `image/png`, `image/jpeg` and
`application/pdf`. Images and documents use their respective content variants;
text/JSON artifact references are for tool results. PNG/JPEG/PDF checks verify
signatures, **not** complete file validity or sanitization. Publishers remain
responsible for content preparation. Each artifact and the aggregate referenced
bytes are capped at 4 MiB; text/JSON also has the tighter JSON/text bounds.
Counting and generation use the same verified bytes, read once per request.

Reasoning output exposes only a summary and an opaque continuation reference.
Append that block unchanged to its assistant turn to continue. Native encrypted
or signed state stays in checksummed private journal records, bound to the
authenticated actor, provider, exact resolution and summary, with a 24-hour expiry.
The filesystem receipt encoding is not additional at-rest encryption. References
are not transferable across actors/models and cannot extend their expiry. A
restart can continue a still-valid record; changing a summary or record fails
closed. Preserve these records with the responses that reference them, including
after expiry, until an explicit retention policy is introduced.

## Streaming completion and cancellation

Use `client.streamModel` with `StreamModelRequest`, containing the same context
and invocation contract as unary calls. Each RPC item wraps its `ModelStreamEvent`
in `event`. Sequence numbers start at 1 and increase without gaps:

```text
started -> contentDelta* -> usageUpdate -> completed
```

Content indices address blocks in the completed normalized response. For Chat,
which has separate text/tool fields, blocks follow their first nonempty fragment
arrival; an empty role/content chunk does not create a phantom block. Tool argument
fragments and reasoning summaries are previews. Consumers must wait for the
validated `completed.response` before executing a tool or accepting structured
output. A refusal remains `CONTENT_FILTER`, not an accepted research proposal.

The host checks lifecycle order, final content against previews, complete JSON,
schemas, final usage and budget before durable publication and completion. It
requires Chat's `[DONE]` and native completion for the other protocols, then EOF;
unknown/trailing events, truncation and cancellation return non-OK RPC status.
SSE decoding caps the response at 8 MiB, 16,384 events and 512 KiB per event; model
content is capped at 256 KiB and 256 blocks. Pull-based decoding retains
backpressure; HTTP/SDK transport buffers still exist within those bounded paths.
Pass an `AbortSignal` to cancel, and always retain a bounded RPC deadline.

An exact completed retry emits `started`, `usageUpdate`, `completed` from the saved
response without regenerating previews or another supplier call. An interrupted
claim remains ambiguous. Request identity is shared between unary and streaming
calls; switching the RPC method does not authorize another charge.

## Duplicate calls and failures

An exclusive, fsynced provider-only claim precedes all supplier requests. The key
is scoped to the authenticated actor. The claim contains a canonical request
fingerprint and cost reserve, not the prompt or API secret. A validated response
is stored privately with a checksum and published without replacing an existing
result. Retrying the exact valid request reads that result, including after
restart, without another supplier request. Normal current authorization, time,
model and policy validation still applies; receipts do not grant access.

A changed request under the same key returns `invocation_conflict`. A prior
claim without a complete, verified result returns `invocation_ambiguous`; it is
never automatically resent. This is conservative duplicate-spend prevention,
not a claim of exactly-once delivery to an external API. A supplier may have
accepted a call whose response was lost. An operator must reconcile uncertainty
before explicitly authorizing a new request/key. Keep claim/result files across
restarts and rollbacks. Never delete them merely to make a retry succeed.

Application errors use non-OK gRPC status with typed `loop.v1.ServiceError` and
allowlisted codes; raw supplier errors, headers and credentials are not echoed.
TLS handshake and malformed transport framing errors remain transport failures.
No operational failure is converted to factor rejection. SIGTERM/SIGINT cancels
in-flight operations; uncertain attempts stay fenced.

## Verification and rollback

```bash
./scripts/pnpm.sh --filter @loop-engine/providerd typecheck
./scripts/pnpm.sh --filter @loop-engine/providerd test
./scripts/pnpm.sh --filter @loop-engine/providerd build
```

Tests use local supplier HTTP fixtures and actual TLS/gRPC sockets; no paid API
call is made. Independent 2/4/8 OS processes exercise journal claims, and real
kill/restart checks cover incomplete and published responses. Tests create their
own `loop-provider-*` temporary directories and remove them after completion.

Unset `PROVIDERD_DEPLOYMENT` and restart to restore health-only operation. Preserve
private invocation/continuation records, audit/research history and configuration
snapshots. There is no SQL migration or production database change. To return to
the text-only build, restore that build's configuration and obtain its own pins;
do not reuse rich-content pins or discard journals. Additive wire fields preserve
the original compatibility baseline. Offline fixtures do not establish paid
supplier availability or `live_verified` status.
