# Compatible, self-hosted and gateway Providers

These routes use the existing authenticated ProviderService and private
deployment described in [native Providers](native-providers.md). Install no
additional SDK, model weight or serving daemon to use the client adapters.
The inference server or gateway is a separately administered deployment.
Offline contracts verify transport behavior; they do not attest a live model,
its weights, entitlement, prices or enabled server features.

## Select a route

| Plugin | Implemented wire | Administrative endpoint |
| --- | --- | --- |
| `openai_compatible` | Chat or Responses | Explicit HTTPS base; Chat/Responses paths are appended |
| `anthropic_compatible` | Messages | Explicit HTTPS base, excluding the final `/v1/messages` |
| `ollama` | Chat | For example `http://127.0.0.1:11434/v1`; anonymous authentication is explicit |
| `vllm` | Chat | Configured server `/v1` base |
| `sglang` | Chat | Configured server `/v1` base |
| `llamacpp` | Chat | Configured llama.cpp server `/v1` base |
| `lmstudio` | Chat | Configured LM Studio server `/v1` base |
| `nim` | Chat | Configured NVIDIA NIM `/v1` base |
| `litellm` | Chat | Configured proxy `/v1` base and fixed upstream mapping |
| `portkey` | Chat | Configured gateway `/v1` base; direct provider/BYOK authentication |
| `openrouter` | Chat | Fixed `https://openrouter.ai/api/v1`; one explicit upstream supplier |

The named server profiles implement Chat. A server's Responses support can use
`openai_compatible` only after verifying the implemented native lifecycle and
capabilities. That generic profile retains its own identity. Naming a backend
does not establish that its installed version supports every OpenAI feature.

Only administrators select `compatible.base_url`. It must be a canonical URL
without a trailing slash, credentials, query or fragment. Paths contain plain
ASCII letters, digits, `.`, `_`, `-` and `/`, with no dot segments. HTTPS is
required except HTTP to the literal loopback addresses `127.0.0.1` and `[::1]`.
HTTP DNS names, including `localhost`, are rejected. A remote private HTTP server
needs a TLS reverse proxy or an explicitly administered loopback tunnel.
Redirects and outbound paths other than the selected generation operation are
rejected. Never place a secret in the URL.

## Configuration

Add a model to the private deployment and its ID to the intended principal's
allowlist. This example requires replacement of capacities, prices and model ID
before it validates. Zero prices are appropriate only for a verified unbilled
service; operational/GPU costs are not inferred from API usage.

```json
{
  "id": "local-maker",
  "plugin": "ollama",
  "model": "REPLACE_WITH_EXACT_RETURNED_MODEL_ID",
  "alias": "research-maker",
  "context_tokens": "REPLACE_WITH_COMBINED_CAPACITY",
  "input_token_limit": "REPLACE_WITH_MAXIMUM_INPUT",
  "output_tokens": "REPLACE_WITH_OUTPUT_LIMIT",
  "input_usd": "REPLACE_WITH_USD_PER_MILLION",
  "output_usd": "REPLACE_WITH_USD_PER_MILLION",
  "cached_usd": "REPLACE_WITH_USD_PER_MILLION",
  "features": {"streaming": true, "tools": true},
  "reasoning": "off",
  "compatible": {
    "base_url": "http://127.0.0.1:11434/v1",
    "wire": "chat",
    "auth": "none"
  }
}
```

`auth: none` forbids `secret_env`; the SDK's placeholder key is removed before
dispatch. `auth: bearer` requires a `LOOP_LLM_*` secret reference and sends only
its Bearer credential. Messages uses `auth: api_key` with `x-api-key`, or explicit
anonymous authentication. Chat/Responses do not accept API-key-header mode in
this profile. Keys are injected through a protected environment, not committed.

`compatible.request_model` optionally selects a deployment name different from
the expected response model. The response still must match top-level `model`.
Use an immutable model/weights revision and stable server mapping. Matching an
echoed name cannot prove that an administrator has not changed the weights.
The endpoint, selector, dialect and prices are part of the resolution digest.
Restart and regenerate `--describe` pins after changing configuration.

Every route requires a verified `input_token_limit`. The Host reserves that
whole input ceiling before generation, rather than inventing a local tokenizer
count. Maximum input plus requested output must fit the declared context.
Missing or inconsistent final token usage cannot produce a successful receipt.

## Capabilities and dialects

Optional `features` remain disabled by default. Enable them only for an installed
model/server combination that implements them. Registered JSON-schema output,
tool arguments and artifact access retain the shared Host checks. Unsupported
hosted tools, audio/video, generated media, citations and opaque Chat reasoning
state fail explicitly. Generic Chat image input accepts private PNG/JPEG with
automatic detail; PDFs use compatible Responses/Messages when supported.

The following `compatible` settings apply only to Chat:

- `strict_tools` defaults to `false`. A caller requesting strict tools is denied
  unless the configured implementation supports native strict enforcement.
  Ollama strict tools are not enabled by this adapter. Local argument validation
  remains mandatory even when native strict mode is available.
- `output_limit` selects `max_tokens` (default) or `max_completion_tokens`, based
  on the server's documented combined output-token behavior.
- `thinking` is `none`, `effort`, or `template`. `none` sends no generic thinking
  control and therefore requires a verified model that does not emit thinking.
  `effort` sends the pinned `reasoning_effort` (`off` becomes `none`; `adaptive`
  becomes `high`). `template` uses `chat_template_kwargs.enable_thinking` and
  accepts only `off`/`adaptive`. These are protocol dialects, not universal model
  capabilities; unsupported effort values must not silently fall back on the server.
- `reasoning_field` selects `reasoning` or `reasoning_content`. Thinking-enabled
  Chat routes require an explicit field and controller. Plain-text thinking is
  saved in private continuation and can be restored after a Host restart.
  llama.cpp additionally receives `reasoning_format: deepseek` when a thinking
  field is selected, or `none` otherwise. Configure its parser/template accordingly.
- `stream_usage` is `separate` (default, requests `include_usage`) or `terminal`
  for servers placing final usage on the finishing content frame. Intermediate
  usage is validated but cannot substitute for a terminal receipt.

Responses/Messages retain their existing native reasoning, private signatures,
cache, tool and schema behavior. Chat-only non-default settings on these wires
are rejected. Thinking-enabled Messages requires native signed thinking, unlike
the separately implemented MiniMax profile. Messages cache creation needs an
explicit `cache_creation_usd` price when caching is enabled.

## Gateway identity and costs

Each gateway requires a `compatible.gateway` object:

```json
{
  "upstream_provider": "openai",
  "route_sha256": "REPLACE_WITH_64_LOWERCASE_HEX_CHARACTERS",
  "maximum_extra_usd": "REPLACE_WITH_PER_REQUEST_USD_CEILING"
}
```

`provider_id` remains `litellm`, `portkey` or `openrouter`. The upstream supplier
and configuration digest are bound into the model catalog; neither is supplied
by a model request. `route_sha256` records the administrator's reviewed routing
configuration, excluding credentials. It is a declaration, not a remote
cryptographic attestation. Archive the corresponding non-secret configuration.

Pin a single model deployment; disable server-side retries, fallback, semantic
cache and dynamic aliases. A repeated request can otherwise produce a different
model result or extra charge hidden behind one gateway response. The Host sends
the controls below and rejects a contradicting returned model; deployment
verification must establish that the gateway actually honors those controls.

- LiteLLM sends `disable_fallbacks: true` and `num_retries: 0`. Also set zero
  retries/no fallback in the proxy, including key/model overrides; select one
  fixed deployment per model group and avoid wildcard/automatic routing.
- Portkey uses the gateway key in `x-portkey-api-key`, the declared supplier in
  `x-portkey-provider`, and a separately referenced BYOK key in Authorization.
  Add `upstream_key_env: LOOP_LLM_PORTKEY_UPSTREAM_KEY` to `gateway`. Both keys
  are required. Inline config sets zero retry attempts; if the response reports
  `x-portkey-retry-attempt-count`, any value other than `0` is rejected. No virtual
  key, mutable hosted config or automatic fallback is selected by this adapter.
- OpenRouter uses one `provider.only` entry, `allow_fallbacks: false` and
  `require_parameters: true`; plugins/search and reasoning are disabled. This
  first profile rejects opaque `reasoning_details`. Dynamic `openrouter/auto`
  and `:online`, `:nitro`, `:floor` selectors are rejected for both request and
  expected response IDs. Use a compatible native wire for supported private
  reasoning, or a dedicated future codec; do not discard opaque state.

`maximum_extra_usd` reserves a verified upper bound for gateway/non-token fees
in addition to token costs. Zero requires evidence of no extra fee. The usage
record's token-based estimate is not an invoice; unreported gateway fees are
not invented as measured charges. The existing per-call budget does not yet
establish a run-wide spending limit (Run Harness is a subsequent phase).

## Verification and rollback

Build/describe/start as in the native guide. Verify model identity, stream
termination, usage, tools and enabled optional capabilities against the deployed
service before allowing research traffic. Do not mark a route `live_verified`
from fixture tests. No model downloads or paid calls are made by these tests:

```bash
./scripts/pnpm.sh --filter @loop-engine/providerd test test/compatible.test.ts test/compatible-state.test.ts
```

Disable compatible routes before rollback; restore the preceding deployment and
binary, regenerate resolution pins and preserve invocation/continuation history.
No database migration or history deletion is needed. Provider namespace/egress
isolation and model catalog hot reload are separate Phase 9 acceptance units.
