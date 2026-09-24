# First-class vendor configuration

These plugins run inside the existing TypeScript Provider Host. Configure the
private listener, certificates, schemas, journal and principal allowlist using
[native Provider conversations](native-providers.md). Each plugin owns a fixed
official destination and protocol profile. A shared SDK transport does not make
the vendor an OpenAI or Anthropic account. No endpoint is accepted from a model
request; regional keys never fall back to another region.

## Routes

| Plugin | Protocol / global base | Suggested secret reference |
| --- | --- | --- |
| `mistral` | Chat, `https://api.mistral.ai/v1` | `LOOP_LLM_MISTRAL_API_KEY` |
| `deepseek` | Chat, `https://api.deepseek.com` | `LOOP_LLM_DEEPSEEK_API_KEY` |
| `qwen` | DashScope Chat, `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` | `LOOP_LLM_QWEN_API_KEY` |
| `xai` | Responses, `https://api.x.ai/v1` | `LOOP_LLM_XAI_API_KEY` |
| `groq` | Chat, `https://api.groq.com/openai/v1` | `LOOP_LLM_GROQ_API_KEY` |
| `together` | Chat, `https://api.together.ai/v1` | `LOOP_LLM_TOGETHER_API_KEY` |
| `fireworks` | Chat, `https://api.fireworks.ai/inference/v1` | `LOOP_LLM_FIREWORKS_API_KEY` |
| `cerebras` | Chat, `https://api.cerebras.ai/v1` | `LOOP_LLM_CEREBRAS_API_KEY` |
| `perplexity` | Sonar, `https://api.perplexity.ai/v1/sonar` | `LOOP_LLM_PERPLEXITY_API_KEY` |
| `glm` | Chat, `https://api.z.ai/api/paas/v4` | `LOOP_LLM_GLM_API_KEY` |
| `kimi` | Chat, `https://api.moonshot.ai/v1` | `LOOP_LLM_KIMI_API_KEY` |
| `minimax` | Messages, `https://api.minimax.io/anthropic` | `LOOP_LLM_MINIMAX_API_KEY` |

Keys are environment-variable references, never literal values in configuration.
All routes require an explicit verified `input_token_limit` and the same pinned
model limits/prices as cloud routes. The host reserves the full input ceiling
before generation; it does not call an incomplete tokenizer an exact counter.
Use the combined input/output capacity for `context_tokens`. Model resolution
pins include vendor settings, prices, capabilities and installed implementation.

Add a model object to the private deployment's `models`, and grant its `id` in
the appropriate principal's `model_ids`. This template deliberately requires
replacement with verified capacities and prices before it validates:

```json
{
  "id": "deepseek-maker",
  "plugin": "deepseek",
  "model": "REPLACE_WITH_VERIFIED_MODEL_ID",
  "alias": "research-maker",
  "context_tokens": "REPLACE_WITH_INTEGER_COMBINED_CAPACITY",
  "input_token_limit": "REPLACE_WITH_INTEGER_MAXIMUM_INPUT",
  "output_tokens": "REPLACE_WITH_INTEGER_OUTPUT_LIMIT",
  "input_usd": "REPLACE_WITH_USD_PER_MILLION",
  "output_usd": "REPLACE_WITH_USD_PER_MILLION",
  "cached_usd": "REPLACE_WITH_USD_PER_MILLION",
  "features": {"streaming": true, "tools": true},
  "reasoning": "adaptive",
  "secret_env": "LOOP_LLM_DEEPSEEK_API_KEY"
}
```

Restart and regenerate `--describe` resolution pins after configuration changes.
Before live use, verify account entitlement, exact returned model ID and the
model's implemented capability subset. A contract-tested plugin is not a live
verification or a promise that all models offered by its vendor are supported.

## Native differences

All routes implement bounded text and streaming. Tools produce proposals for the
caller; the Provider never executes them. Registered schemas still receive local
validation. Explicit strict-tool requirements are rejected on profiles that do
not implement native strict mode, rather than silently removing the requirement.

| Plugin | Implemented differences and limits |
| --- | --- |
| Mistral | Thinking content chunks, signatures and closure flags are retained in private continuation. Text-only thinking chunks are implemented; reference chunks fail explicitly. `service_tier: standard_only`. Strict tools denied. |
| DeepSeek | Separate `reasoning_content` and native cache hit/miss accounting. Thinking/tool calls omit `tool_choice`; forced/named selection while thinking is denied. Native JSON-schema output, strict functions and vision are not advertised in this profile. |
| Qwen | Native `enable_thinking`, optional effort and regional routing. Search disabled. Thinking unary calls consume a native stream and return only validated completion. `max_completion_tokens` includes thinking; reserve ten tokens for the documented possible overshoot. Budgets of ten output tokens or fewer fail before dispatch. Strict tools and schema-plus-image requests are denied. |
| xAI | Native Responses with `store: false`, encrypted thinking continuation, function tools, registered JSON schemas and images. Hosted tools are not enabled. |
| Groq | Separate `reasoning` field; GPT-OSS uses `include_reasoning`, other supported models use parsed/hidden reasoning. Terminal usage may arrive in `x_groq.usage`. |
| Together | Separate `reasoning_content` by default; select `vendor.reasoning_field: reasoning` for models using that documented field. GLM thinking requests retain prior thinking. |
| Fireworks | Refuse context truncation; preserve thinking through `thinking.keep: all`; native strict tools and JSON schemas. Effort/boolean reasoning controls are model-dependent. |
| Cerebras | Separate `reasoning`, parsed/hidden mode and native output ceiling. Vision is not enabled. |
| Perplexity | Sonar text/JSON-schema profile with `disable_search: true`. Functions, thinking and vision denied. A verified per-call non-token USD upper bound is mandatory. Search output or search usage causes failure; use a model/account combination that honors disabled search. |
| GLM | Native thinking preservation, auto-only tool selection, tool-stream flag and `sensitive` refusal normalization. Strict functions and JSON-schema output denied; JSON mode is not equivalent to schema enforcement. |
| Kimi | K2-family thinking preservation; K2.7 cannot disable thinking. K3 uses native effort and cannot disable thinking. K2 forced/named tools and explicit effort are denied. Strict functions are not enabled. |
| MiniMax | Native Messages. Preserve signed or unsigned thinking exactly; automatic cache read/write accounting requires explicit cache-creation pricing. M2-family thinking cannot be disabled and vision is denied. M3 image input is available. Strict functions, PDFs and registered schema output are not enabled. |

Optional image input uses private PNG/JPEG artifacts with automatic detail;
Chat vendor profiles reject explicit high/low detail rather than discard it.
PDF, audio, video, generated media, hosted search/code and unknown content unions
are outside these implemented profiles. Configurations may narrow capabilities,
but cannot enable capabilities absent from the profile.

`reasoning` is a pinned model setting, not a per-request raw vendor parameter.
`adaptive` maps to the vendor's implemented enable/default mode. Explicit effort
must also be supported by the chosen model. Missing thinking-token measurement
does not establish zero internal computation; unreported subsets remain zero
in the shared usage record, while reported total output remains authoritative.

## Regions and extra charges

Optional `vendor.region` defaults to `global`. The only implemented alternatives:

- Qwen: `cn` uses `dashscope.aliyuncs.com`; `us` uses
  `dashscope-us.aliyuncs.com`. `vendor.workspace` selects the official
  `<workspace>.<location>.maas.aliyuncs.com` base for global (`ap-southeast-1`),
  China (`cn-beijing`) or Japan (`ap-northeast-1`). Japan requires a workspace;
  US plus workspace is rejected. Keys must belong to the selected region.
- GLM `cn`: `https://open.bigmodel.cn/api/paas/v4`.
- Kimi `cn`: `https://api.moonshot.cn/v1`.
- MiniMax `cn`: `https://api.minimax.cn/anthropic`.

For Sonar set `vendor.maximum_extra_usd` to a verified non-token per-request
upper bound. The host adds it to the token reservation before dispatch. This
administrator-supplied bound is not a billing measurement. Search must remain
disabled; do not configure a hosted research model that requires search. The
shared `charged_cost` field remains absent without supplier billing evidence.

SDK retries and redirects are disabled. Errors are redacted. Cancellation and
transport loss retain an ambiguous invocation claim; operators cannot safely
assume the vendor did no work. Replaying the same completed invocation does not
create another paid request. Durable run-wide budgets remain a Harness concern.

## Verification and rollback

The contract suite uses actual TLS/gRPC, SDK transports and local vendor HTTP
fixtures; it does not make paid calls. See
[unit 5 evidence](../verification/phase-09-vendor-plugins.md).

To roll back, disable vendor routes, restore the prior binary/configuration,
and regenerate pins. Preserve journals and private continuation records. Older
binaries reject new plugin IDs; no database migration is required.
