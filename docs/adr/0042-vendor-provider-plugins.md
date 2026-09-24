# ADR 0042: First-class vendor plugins

Status: accepted on 2026-09-24. Chinese task commit `e1055c6` is pushed; local
gates and all seven jobs in exact-commit CI `35953153780` passed.
Phase 9 delivery unit 5; the entire phase remains in progress.

## Requirement and decision

Expose Mistral, DeepSeek, Qwen/DashScope, xAI, Groq, Together, Fireworks,
Cerebras, Perplexity, GLM/Zhipu, Kimi/Moonshot and MiniMax through the existing
authenticated ProviderService. A vendor owns its endpoint, authentication,
request dialect, supported capabilities, usage and error normalization even
when its official protocol shares an OpenAI or Anthropic wire format.

Reuse the installed SDK transports and validated native codecs. Keep cloud and
vendor logic in providerd; no vendor branch belongs in an Agent or research
loop. Model configuration may narrow an implemented capability, never invent
one. Bind administrative regions, thinking dialects and ancillary cost bounds
to the catalog snapshot. Requests cannot choose endpoints or raw vendor options.

Preserve native reasoning fields privately across tool turns. Normalize usage
only from measured supplier fields, and reject missing usage or conflicting
totals. Vendor-specific terminal usage must still precede a validated stream
completion; intermediate usage is not a terminal receipt. Unsupported hosted
tools, generated media and output variants fail closed. Perplexity's Sonar
profile disables remote search and separately reserves non-token charges.

Use the existing full-input-capacity reservation where an exact complete-request
counter is unavailable. Do not silently drop strict-tool or schema requirements
on a protocol that implements only JSON mode. Official protocol compatibility is
not evidence of model availability, account entitlement or a successful live call.

Mistral native thinking chunks retain their signatures and closure flags;
stream deltas reconstruct that private state without exposing signatures over
the service protocol. MiniMax permits unsigned thinking without weakening the
original Anthropic signature requirement. Qwen thinking unary calls consume its
native stream because some thinking models are stream-only. Its combined output
ceiling reserves the documented ten-token overshoot. Unknown native thinking
variants, incompatible tool requirements and undeclared charges fail explicitly.

## Acceptance and rollback

Exercise every plugin through actual TLS/gRPC and installed SDK HTTP transports
against local fixtures. Cover vendor request fields, exact origins, streaming,
tools, registered schemas, reasoning continuation, cache usage, errors, missing
credentials and capability/price failures. Preserve existing native/cloud cases.

Disable vendor routes before reverting to the prior binary/configuration.
Preserve invocation claims, results and private continuation records. No SQL
migration or historical rewrite is introduced. Publish one Chinese task commit
with configuration guidance and observed verification evidence.

## Official references

Protocol profiles were checked against primary documentation on 2026-09-23.
They describe implemented subsets; upstream availability remains model-specific.

- [Mistral thinking and replay](https://docs.mistral.ai/studio/conversations/reasoning)
- [Mistral signed thinking type](https://github.com/mistralai/client-ts/blob/main/src/models/components/thinkchunk.ts)
- [DeepSeek thinking](https://api-docs.deepseek.com/guides/thinking_mode/)
- [DeepSeek agent integration](https://api-docs.deepseek.com/quick_start/agent_integrations/oh_my_pi/)
- [Qwen native Chat parameters](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-chat-completions)
- [Qwen structured output limitations](https://help.aliyun.com/en/model-studio/qwen-structured-output)
- [xAI Responses and encrypted continuation](https://docs.x.ai/developers/model-capabilities/text/generate-text)
- [Groq reasoning](https://console.groq.com/docs/reasoning)
- [Together thinking fields](https://docs.together.ai/docs/inference/chat/reasoning)
- [Fireworks native Chat](https://docs.fireworks.ai/api-reference/post-chatcompletions)
- [Cerebras native Chat](https://inference-docs.cerebras.ai/api-reference/chat-completions)
- [Perplexity Sonar](https://docs.perplexity.ai/api-reference/sonar-post)
- [GLM Chat and tool restrictions](https://docs.z.ai/api-reference/llm/chat-completion)
- [Kimi model capabilities](https://platform.kimi.ai/docs/api/models-overview)
- [MiniMax Messages](https://platform.minimax.io/docs/api-reference/text-anthropic-api)
- [MiniMax China endpoint](https://platform.minimax.cn/docs/api-reference/text-anthropic-api)
