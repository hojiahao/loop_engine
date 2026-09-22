# Native Provider invocation

Phase 9 unit 1 adds an optional TLS 1.3 / HTTP/2 gRPC listener to the existing
TypeScript `providerd`. It implements `loop.provider.v1.ProviderService/InvokeModel`
for three native paths: `openai_responses`, `openai_chat`, and `anthropic`.
The bootstrap HTTP health endpoint remains at `127.0.0.1:8090` and cannot invoke
a model. No listener is enabled by an API key alone.

This delivery supports unary system/user/assistant **text** conversations ending
with a user message, plus text/refusal responses and explicit length termination.
All system messages must precede conversational messages. Tools, JSON-schema
output, thinking continuation, images/documents and streaming are explicitly
unavailable until delivery unit 2; no artifact path is resolved. Effective model
capability snapshots reflect this subset. Other suppliers, model discovery and
catalog reload are later Phase 9 units. Run-wide scheduling/budgets remain Phase
10/11. This is not a completed autonomous research loop.

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
native model ID must equal the configured ID; mutable supplier aliases that
resolve differently fail closed. `input_usd`, `output_usd` and `cached_usd` are
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
count. Anthropic uses its native Messages counter. Counters may differ from final
usage; an observed overrun is an explicit failure, not a reversible supplier
charge. Maximum output tokens are also sent to the supplier.

The preflight reserve uses the full input/output allowance and uncached pinned
prices with integer nanodollar arithmetic, rounded up. No cache discount is
assumed in that reserve. Reported cached input tokens are included in total input
tokens for both suppliers. Cache creation is not requested; unexpected Anthropic
cache writes are denied because their separate pricing is not represented here.
`ModelUsage.charged_cost` remains absent: token-price calculations are not invoices.
These are per-call limits; do not treat them as a durable cumulative run budget.

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
the private journal, audit/research history and configuration snapshots. There is
no SQL migration or production database change. The prior executable cannot serve
new native calls; do not discard journals when deploying it.
