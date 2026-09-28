# Cloud Provider deployments

Azure OpenAI, Google Vertex AI and AWS Bedrock use the same private deployment
file, mTLS listener, `InvokeModel`/`StreamModel` RPCs, journal and model resolution
pins as the [native Provider service](native-providers.md). Follow that guide for
certificates, registered schemas, prompt artifacts, budgets and startup commands.
The cloud adapters are protocol implementations, not evidence that an account
has cloud IAM permission, a deployed model, quota or paid access.

## Common configuration

Add a model entry and add its `id` to the approved principal's `model_ids`.
Every entry needs `id`, `plugin`, `model`, `alias`, `context_tokens`,
`input_token_limit`, `output_tokens`, `input_usd`, `output_usd` and `cached_usd`.
Enable optional `features` only after checking the exact deployment's support.
Use versioned model IDs and verified prices, never sample prices from a guide.

`model` is the resolved native model identity. `cloud` contains the separate
deployment selector and location. Both are pinned in the catalog digest; changing
a deployment, region, identity mode or model requires a restart and fresh
`--describe` pins. A request cannot supply an endpoint or credential. Secret
references are environment-variable **names**, never values. Keep configuration
at mode `0600` outside Git and inject secrets through the service's protected
environment or workload identity, not shell arguments.

These cloud routes reserve `input_token_limit`, the verified **full vendor input
capacity**, before generation. This is deliberately conservative and is not a
measured token count. The caller's input and dollar budgets must cover this bound
and its output allowance. Do not reduce the declared vendor capacity to squeeze
under a small budget. Actual usage is independently checked against both caller
limits and combined context capacity. A network or counting failure never
triggers a guessed-count fallback. Supplier cost is not an invoice, and a local
deadline cannot undo a request already accepted by a supplier.

## Azure OpenAI

Use `azure_responses` or `azure_chat`. This implementation uses the public-cloud
GA `/openai/v1` API, without a required dated `api-version`. It does not silently
translate a legacy dated-preview endpoint into v1.

Place this `cloud` object in the model entry and set
`secret_env: "LOOP_LLM_AZURE_API_KEY"` for API-key authentication:

```json
{
  "kind": "azure",
  "resource": "your-resource-name",
  "domain": "openai.azure.com",
  "deployment": "your-deployment-name",
  "auth": "api_key"
}
```

`domain` may also be `services.ai.azure.com`; arbitrary URLs, sovereign domains,
paths and query strings are rejected. The SDK sends `cloud.deployment` as the
request's `model`; a response must echo the configured resolved `model`, not
the deployment alias. Pin the actual deployment's model/version in Azure and
verify that mapping before enabling it. Local pins cannot prevent an Azure
administrator from changing the remote deployment.

For Entra authentication set `auth` to `entra` and **omit** `secret_env`.
The official `@azure/identity` `DefaultAzureCredential` obtains a token for
`https://ai.azure.com/.default`. It supports the SDK's environment credentials,
workload/managed identity and local development credential chain. Configure the
intended service identity and model permissions in Azure; do not rely on an
unintended interactive developer login on a production host. Entra uses
`Authorization: Bearer`; key mode uses `api-key`, never both. Tokens refresh via
the SDK and do not enter catalog snapshots, receipts or error messages.

Responses reuses native OpenAI tools, registered schemas, images/PDFs, reasoning
continuation and streaming. Chat keeps its existing `reasoning: "off"` profile.
Capabilities remain deployment/model dependent.

Official reference: [Azure v1 lifecycle and identity examples](https://learn.microsoft.com/en-us/azure/foundry/openai/api-version-lifecycle).

## Google Vertex AI

Use `vertex_generate`, a bare versioned Gemini `model` ID, and omit `secret_env`:

```json
{
  "kind": "vertex",
  "project": "your-gcp-project",
  "location": "us-central1"
}
```

The adapter derives the official regional origin, v1 project/location path and
`publishers/google/models/<model>` selector. `location: "global"` uses
`aiplatform.googleapis.com`; it is an explicit administrator choice, not a
regional fallback. Google Application Default Credentials (ADC) are resolved
through `google-auth-library` with the `cloud-platform` scope. In a deployment,
use workload identity/attached service identity with the required Vertex access;
for local development use an authorized ADC configuration. If a credential file
is required, `GOOGLE_APPLICATION_CREDENTIALS` refers to a protected file outside
Git. No credential JSON belongs in the model entry.

Calls use OAuth, not a Gemini Developer API key. Explicit project/location
configuration prevents an ambient Gemini API key from selecting express mode.
The native `modelVersion` must equal the pinned model ID. Google tools, verified
inline images/PDFs, structured output, signed Parts and streaming reuse the
GenerateContent implementation. The implemented thinking dialect is effort-level
thinking; do not advertise it for a model requiring another dialect. Vertex
partner-model APIs and Interactions are not represented by this plugin.

Official reference: [Vertex SDK and ADC usage](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/samples/googlegenaisdk-textgen-with-multi-local-img).

## AWS Bedrock

Use `bedrock_converse`, a resolved foundation `model` ID and omit `secret_env`:

```json
{
  "kind": "bedrock",
  "region": "us-east-1",
  "model_id": "REPLACE_WITH_FOUNDATION_OR_INFERENCE_PROFILE_ID",
  "reasoning_dialect": "none",
  "credentials": {
    "access_key_env": "LOOP_LLM_AWS_ACCESS_KEY_ID",
    "secret_key_env": "LOOP_LLM_AWS_SECRET_ACCESS_KEY",
    "session_token_env": "LOOP_LLM_AWS_SESSION_TOKEN"
  },
  "guardrail": {
    "id": "REPLACE_WITH_GUARDRAIL_ID",
    "version": "REPLACE_WITH_PUBLISHED_NUMERIC_VERSION",
    "maximum_usd": "REPLACE_WITH_VERIFIED_PER_CALL_GUARDRAIL_BOUND"
  }
}
```

This illustrative object requires real deployment selectors; omit `guardrail`
when none is configured. Guardrail versions must be published positive integers,
not `DRAFT`. Omit `session_token_env` only for credentials that do not have a
session token. If a reference is declared but absent, invocation fails; it does
not fall back to a different identity.

Guardrail charges are separate from model-token prices. `guardrail.maximum_usd`
is a mandatory administrator-verified worst-case per-call USD reservation for
the configured guardrail policies and permitted input/output sizes. The host
adds it to the token reservation before dispatch; it is not sent as an AWS
parameter or fabricated as observed usage. Include it in the caller's USD budget.
Without a defensible bound, omit the guardrail route rather than price it at zero.
Cloud invoices, taxes, network and provisioned-capacity charges are not measured
by model usage, and still require account-level billing controls.

Alternatively omit `credentials` to select the official AWS default credential
chain (environment, configured profiles/SSO, web identity, container or instance
role). An explicit region and official endpoint override ambient endpoint
configuration. The request uses SDK SigV4 with service `bedrock`, including a
session token where applicable; an ambient Bedrock bearer token does not replace
SigV4. Credential refresh belongs to the official SDK. Prefer scoped workload
roles for deployment and keep any private profile/cache outside Git.

`cloud.model_id` may identify a foundation model, inference profile, application
inference profile or provisioned model. Supported ARNs must match the configured
region. Prompt-management and prompt-router selectors are denied. Public AWS
commercial regions are implemented; China/GovCloud endpoint partitions are not
silently guessed. A cross-region inference profile may execute outside the
endpoint region; the configured endpoint alone is not a data-residency guarantee.
Bedrock Converse does not echo an independently resolved model version. Its
receipt binds the administrator's declared selector/model mapping, not a verified
backend revision. Confirm that mapping and model access in AWS.

Converse translates system/conversation blocks, client tool calls/results,
verified inline images/PDFs and native `outputConfig.textFormat` JSON-schema
output. Bedrock's supported schema subset and feature support vary by model;
for example, native structured output does not support numeric `minimum`.
Register a compatible schema; supplier rejection remains a failed call.
With declared tools, `ToolChoice.NONE` has no lossless Converse mapping and is
rejected before dispatch. Without tools, ordinary text calls remain available.
Forced/named selection is denied while thinking is enabled.

For supported Anthropic foundation models use `reasoning_dialect: "anthropic"`
with top-level `reasoning: "adaptive"` or `reasoning: "enabled"` plus a
`thinking_tokens` budget. Other reasoning dialects are denied. Signed reasoning
and redacted bytes remain in the actor/model-bound private continuation store;
they are restored on a later tool turn without exposing signatures through RPC.
Hosted/server tools, generated media and citations are outside this profile.

When `features.prompt_caching` is enabled, configure `cache_creation_usd` and
verify the model accepts explicit five-minute checkpoints. The adapter adds one
checkpoint after static system content (or the last user content if no system
content exists). Only five-minute writes are priced here; unexpected one-hour
usage fails. Total input is uncached input plus cache reads and writes; total
usage must reconcile. Thinking remains part of output and is not assigned an
invented separate token count.

ConverseStream is AWS binary EventStream, not SSE. The adapter checks frame
length, CRC, event names, output unions and lifecycle before completion. Streams
are bounded to 8 MiB, 16,384 events and 512 KiB per frame. Unknown events cannot
be silently skipped by the SDK. Cancellation, malformed bytes, missing usage,
post-terminal events or a broken connection cannot produce a successful receipt.

Official references: [Converse](https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_Converse.html),
[ConverseStream](https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_ConverseStream.html),
[structured output](https://docs.aws.amazon.com/bedrock/latest/userguide/structured-output.html),
[prompt caching](https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-caching.html).

## Verification, operation and rollback

`--describe` validates configuration and prints the pinned model/policy snapshots;
it does not acquire tokens, invoke models or attest entitlement. Cloud identity
resolution is bounded by the same operation deadline as generation. An identity
SDK may finish a non-cancellable internal lookup later; it cannot then dispatch a
cancelled model request. Generation retries are disabled on all routes. A client
must not erase an ambiguous journal claim to try again after a timeout.

The local contracts use fake credentials, actual SDK HTTP encoding/signing,
TLS/gRPC, local vendor fixtures and independent signature verification. OAuth
fixtures inject local token resolvers; they do not claim a real Entra/ADC login or
refresh test. No adapter is `live_verified` without a credentialed, explicitly
budgeted cloud smoke test. No live cloud credential is required by offline CI.

To stop new cloud work remove the cloud route IDs from the approved configuration
and restart the service. Before reverting the task commit, restore a deployment
supported by the previous binary and regenerate its model pins. Preserve all
invocation and continuation records, including uncertain calls. There is no SQL
migration, research-state change or holdout unlock to reverse.
