# ADR 0040: Google and Cohere native protocols

Status: implemented; local check/test/build passed; `d5d49de` committed and pushed.
All seven jobs in exact-commit CI `35830578199` passed. Phase 9 delivery unit 3
is complete.

## Requirement and boundary

Expose Google GenerateContent, Google Interactions and Cohere V2 Chat through
the existing authenticated ProviderService, including native streaming and
client-managed function calls. They remain separate pinned protocol routes;
the orchestration and numerical services do not import vendor code. Reuse the
existing request validation, private prompt store, schema registry, journal,
deadlines, cancellation and completion receipts.

Use exact versions of the official TypeScript SDKs. Disable SDK retries and
redirects and retain bounded response parsing. Google Interactions uses the
current step-based protocol with `store: false`; no hosted agents, background
jobs or provider-managed conversation IDs are exposed. Unsupported features
and protocol changes fail explicitly, before a successful receipt is written.

## Input accounting

GenerateContent uses the native countTokens endpoint with the entire generation
request, including instructions and tool declarations. The count is a vendor
preflight count, not evidence of a supplier charge.

Cohere's tokenize endpoint counts raw text, not the full chat envelope. Google
Interactions does not provide a complete request counter in this SDK. For these
routes require a pinned `input_token_limit` and reserve that entire vendor input
ceiling before dispatch. Do not
label a local text estimate as a measured chat count. The caller's input budget
and monetary reservation must cover that ceiling; actual response usage is
checked independently. This trades utilization for a conservative budget bound.
The full bound is not an observed request size: do not reject a shared-context
model merely because its full input capacity plus maximum output cannot fit
simultaneously. The supplier can reject an oversized actual request; final usage
must fit the pinned combined capacity and both caller limits. A falsely reduced
administrator declaration is not a verified supplier limit. Catalog discovery
and live entitlement evidence remain separate Phase 9 units.

Cohere `usage.tokens` is actual model usage; `billed_units` is a different billing
quantity, not cached token evidence. Supplier charged cost remains absent.
Only Cohere's explicit `cached_tokens` establishes the cached subset; its thinking
tokens are not separately measured in this response profile. Native thinking
content can be continued privately when enabled in the pinned model route.
Google thought tokens count toward output and are reported separately as a
subset, according to each protocol's usage fields.

## Capability and identity differences

Google GenerateContent preserves whole signed Parts in the existing private
continuation store and binds their visible text/tool projection on replay.
Interactions preserves standalone thought steps and their signatures. Cohere
retains native thinking content; tool plans are ordinary assistant text. Existing
actor/model/expiry bindings apply to all three. No new state store is introduced.

Google supports inline verified image/PDF artifacts with automatic image detail;
Cohere supports images and denies PDF capability. Cohere named tool selection,
mixed per-tool strictness and structured output combined with tools are denied
before dispatch. The new Google routes use the SDK's effort-level thinking
configuration; models requiring a different thinking dialect must not be falsely
advertised. Hosted tools, image thought summaries and generated media are outside
this text/function research profile. Unknown output features fail closed.

Google model-version echoes must match the pinned bare model ID. Cohere does not
return a resolved model ID in V2 Chat: its receipt binds the requested versioned
ID but cannot independently attest an internal supplier revision. Keep that
limitation visible instead of manufacturing a model echo.

Dependencies are exact-pinned `@google/genai` 2.24.0 and `cohere-ai` 8.1.0. SDK
retries are disabled. The published packages are already built: explicitly deny
Google's no-op preinstall and protobufjs's version-warning-only postinstall under
the existing pnpm build policy. Do not enable unrelated install scripts.

## Acceptance and rollback

Exercise the installed SDKs over local HTTP vendor fixtures through the actual
TLS/gRPC service, including rich requests, native stream boundaries, missing
usage, malformed output, cancellation, budget denial and vendor errors. These
contracts establish offline protocol verification, not live model availability.

Disable the three new deployment routes to stop new writers. Existing receipts
and immutable audit evidence remain readable. Revert this task commit only
after removing its plugin IDs from deployment configuration. No database or
research-history migration is required.

## Official protocol references

- <https://ai.google.dev/api/generate-content>
- <https://ai.google.dev/api/tokens>
- <https://ai.google.dev/api/interactions-api>
- <https://ai.google.dev/gemini-api/docs/thinking>
- <https://docs.cohere.com/reference/chat>
- <https://docs.cohere.com/reference/chat-stream>
- <https://docs.cohere.com/reference/tokenize>

Capability restrictions and executable evidence are recorded with the completed
delivery unit; no route is marked live-verified without credentialed testing.
