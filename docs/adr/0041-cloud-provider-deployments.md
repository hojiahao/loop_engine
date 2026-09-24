# ADR 0041: Cloud Provider deployments

Status: accepted; task commit `5594ea8` is pushed and exact-commit CI
`35842723030` passed all seven jobs. Phase 9 delivery unit 4.

## Requirement and decision

Expose Azure OpenAI v1 Responses/Chat, Vertex AI Gemini GenerateContent and AWS
Bedrock Converse/ConverseStream through the existing authenticated ProviderService.
Keep deployment authentication separate from caller metadata and reuse the
existing journal, budgets, schemas, prompt store and private continuation records.
No Agent-loop branch, service, database table or research permission is added.

Deployment configuration pins the public cloud resource, region/location,
deployment or inference-profile ID, resolved model ID and identity mode. These
values contribute to the model catalog fingerprint. Callers cannot override
endpoints, credentials, native parameters or deployment mapping. Azure uses GA
`/openai/v1`, without a dated preview API; Vertex uses the Google publisher's v1
GenerateContent API. Sovereign clouds and Vertex partner protocols are not
silently treated as these implemented routes.

Use official SDKs for cloud identity and AWS SigV4/event decoding. Explicit API
keys are supported for Azure; Entra and Google ADC support workload identities.
Bedrock supports explicit credential environment references or the official
default credential chain. Credentials never enter model snapshots or receipts.
Credential resolution is deadline bounded; cancellation prevents later model
dispatch even when an identity SDK cannot cancel its own internal lookup.

Azure requests name a deployment while replies must name the configured model.
Vertex retains the native model-version check. Bedrock does not echo a resolved
model version: its mapping is an administrator declaration, not independent
attestation. A deployment administrator must verify and pin that mapping; a
mutable cloud deployment cannot be made immutable by a local identifier.

For cloud routes reserve the configured full vendor input ceiling. Neither an
Azure counter nor a uniform Bedrock/Vertex counter includes every supported
request field for every model. Do not silently fall back from a failed counter,
or call a local estimate a measured count. Final usage still must fit the caller
budget and declared combined context. Use the existing conservative-ceiling
contract from ADR 0040 and document its utilization tradeoff.

Reuse OpenAI/Google wire codecs; Bedrock owns its native content union and binary
event stream. Bound decoded bytes, frames and events, validate CRC and lifecycle,
require terminal usage, and reject unknown output. SDK generation retries and
redirects remain disabled. Guardrails are pinned configuration, never model input.
Only implemented reasoning/cache dialects may be enabled.

Guardrails require a separately pinned per-call USD bound, added to the token
reservation before any dispatch. This is an administrator-verified upper bound,
not a measured guardrail charge or an invoice. It avoids implicitly pricing an
enabled ancillary service at zero without adding a general billing framework.

## Acceptance and rollback

Exercise real TLS/gRPC and local vendor HTTP fixtures. Verify AWS signed method,
path, body, region and session token; Azure key/Bearer separation and deployment
mapping; Vertex project/location and OAuth headers. Cover rich content, streams,
identity failures, cancellation, redirects, malformed/truncated binary events,
missing usage, capability denial and catalog changes. Offline verification is
not evidence of IAM permission, cloud entitlement or a paid live call.

Disable the new routes before reverting this task. Preserve immutable invocation
and continuation files, including ambiguous claims. Older binaries reject the
new plugin IDs. No destructive migration or historical rewrite is required.

## Official references

- <https://learn.microsoft.com/en-us/azure/foundry/openai/api-version-lifecycle>
- <https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_Converse.html>
- <https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_ConverseStream.html>
- <https://docs.cloud.google.com/vertex-ai/generative-ai/docs/samples/googlegenaisdk-textgen-with-multi-local-img>
- <https://developers.openai.com/api/docs/guides/token-counting>
