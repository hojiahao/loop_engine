# ADR 0043: Compatible, self-hosted and gateway routes

Status: implemented; 172 new cases, complete TypeScript regression, `just check`
and workspace build passed on 2026-09-24. Publication and exact-commit CI remain
required. Phase 9 delivery unit 6.

## Requirement and decision

Connect OpenAI/Anthropic-compatible endpoints, Ollama, vLLM, SGLang, llama.cpp,
LM Studio, NVIDIA NIM, LiteLLM, Portkey and OpenRouter through the existing
authenticated ProviderService. Reuse installed SDKs, bounded transports and
validated native lifecycles; do not add another service or provider logic to
Rust/Python. The preceding vendor task is committed and pushed as `e1055c6`;
all seven remote gates passed in exact-commit CI `35953153780`.

Only private administrative configuration can select a base URL, wire protocol,
credential mode, deployed model selector and supported dialect. HTTPS is required
except explicit loopback HTTP; no userinfo, query, fragment or redirect is allowed.
Anonymous local endpoints are explicit and receive no dummy authorization header.
Caller messages never choose addresses or override native parameters.

Separate gateway identity from its declared upstream supplier. Bind upstream
mapping and the administrator's gateway configuration digest into the catalog.
Disable automatic fallback where the gateway protocol provides that control.
An administrator declaration is not independent proof of the remote routing
configuration; document any required server-side restrictions and reject output
that contradicts the pinned model. Do not claim that a gateway is its upstream.

Capabilities are opt-in and may not exceed implemented codecs. Generic Chat
profiles support documented, explicitly selected reasoning fields; unsupported
opaque state or hosted tools fail closed. Native-compatible Responses/Messages
retain their full existing private-continuation semantics. Exact model/server
entitlement and runtime configuration require deployment verification.

## Acceptance and rollback

Exercise every route through TLS/gRPC and actual SDK/local HTTP transports.
Cover text/stream/tool/schema flow, configured auth versus anonymous endpoints,
gateway routing controls, reasoning continuation, request/response identity,
redirects, unknown capability, malformed output, usage, cancellation and replay.
Do not download model weights or make paid calls as part of offline contracts.

Disable the new routes before reverting to the preceding binary/configuration.
Preserve invocation claims/results and continuation records. No SQL migration
or audit-history rewrite is required. Complete one task commit with operational
configuration, observed evidence and a Chinese subject before the next unit.

## Primary references

Reviewed on 2026-09-24. Each backend's installed version and model features still
need deployment verification; this ADR does not promise every server feature.

- [Ollama compatibility](https://docs.ollama.com/api/openai-compatibility)
- [vLLM serving](https://docs.vllm.ai/en/latest/serving/online_serving/openai_compatible_server/)
- [vLLM reasoning](https://docs.vllm.ai/en/latest/features/reasoning_outputs/)
- [SGLang Chat](https://docs.sglang.io/docs/basic_usage/openai_api_completions)
- [llama.cpp server](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)
- [LM Studio compatibility](https://lmstudio.ai/docs/developer/openai-compat)
- [NVIDIA NIM API](https://docs.nvidia.com/nim/large-language-models/latest/api-reference.html)
- [LiteLLM fallback and retry controls](https://docs.litellm.ai/docs/proxy/reliability)
- [Portkey authentication headers](https://portkey.ai/docs/api-reference/inference-api/headers)
- [Portkey retry receipts](https://portkey.ai/docs/product/ai-gateway/automatic-retries)
- [OpenRouter provider routing](https://openrouter.ai/docs/guides/routing/provider-selection)
