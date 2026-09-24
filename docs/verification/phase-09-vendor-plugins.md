# Phase 9 unit 5: First-class vendor plugins

Status: implemented; local quality gates, vendor contracts, CLI retest and
workspace build passed on 2026-09-24. Task publication and exact-commit CI
remain required. Phase 9 is not complete.

## Requirement and implementation

ADR 0042 connects twelve independently identified suppliers to the existing
authenticated ProviderService: Mistral, DeepSeek, Qwen, xAI, Groq, Together,
Fireworks, Cerebras, Perplexity, GLM, Kimi and MiniMax. xAI uses native Responses,
MiniMax uses Messages, and the remaining plugins translate their official Chat
dialects. No Agent-loop supplier branch, database migration, research privilege,
service or dependency is added.

Profiles fix official origins, accepted regions, authentication, thinking/tool
controls, structured-output limits and measured usage semantics. Shared SDKs
retain their native transport/error handling with generation retries disabled.
All routes use existing mTLS actor checks, pinned resolution, deadline, budget,
journal, schema and private-continuation boundaries. Administrative settings may
narrow implemented features but cannot add unsupported capabilities.

Mistral retains thinking signatures/closure state through unary and streamed
turns; MiniMax unsigned thinking does not relax native Anthropic validation.
DeepSeek preserves its thinking/cache fields and omits forbidden tool selection
while thinking. Qwen uses a combined output ceiling with the documented ten-token
overshoot allowance; thinking unary requests consume a validated native stream.
Groq terminal usage and conflicting usage fields receive explicit checks. Sonar
disables search and requires a separately declared non-token cost upper bound.
China/US/Japan routing is explicit; no regional credential fallback exists.

## Executable cases

The 153 new cases in `vendor.test.ts` and `vendor-state.test.ts` exercise installed
SDKs through real TLS/gRPC and local HTTP fixtures:

- All twelve plugins: unary text, native destination/authentication/parameters,
  ordered streaming, missing credentials, missing usage, redacted HTTP 429,
  and truncated streams that cannot publish a successful receipt.
- Eleven function-capable routes: tool proposal/result round trips and streamed
  argument fragments. Nine schema-capable routes: native schema request and
  locally validated structured response.
- Eleven thinking-capable routes: unary/stream continuation restored through a
  fresh Host, provider identity, private signature state and tamper rejection.
- Mistral signed chunks and unsupported reference rejection; DeepSeek thinking
  tool selection; Groq conflicting terminal receipts; cache/total/reasoning
  accounting conflicts; unsupported strict-tool requests rejected pre-dispatch.
- Three distinct wire families: cancellation and ambiguous replay without another
  generation request. Sonar non-token budget reservation and unexpected search
  output/charges. Six regional origin paths plus model-catalog dialect pinning.

Existing native/cloud cases retain compiled CLI, schema/media isolation,
cross-process journal writers and crash recovery. These are offline fixtures,
not actual model-generation results or evidence of account entitlement.

## Observed verification

On 2026-09-24, the TypeScript workspace run passed all 119 protocol cases and
421 of 422 Provider cases. The compiled CLI startup case exceeded its 15-second
test deadline while the host had exhausted disk space. After project-owned
temporary-build cleanup, the unchanged CLI file passed all three isolated cases
(17.42 seconds including import/startup). All 153 new vendor cases passed in the
workspace run. Neither a longer timeout nor a skipped assertion was used to
hide the first failure.

`just check` passed: 3,919 Python/Rust/Shell and 589 TypeScript/JavaScript function
declarations meet the naming rule; protocol generation/compatibility and wire
fixtures match; Rust fmt/Clippy (`-D warnings`), TypeScript format/lint/types and
all four Python environment checks passed. TypeScript protocol, Provider and Web
builds passed. The Web bootstrap has no behavior tests and is not UI acceptance.
No Rust/Python behavior changed; the exact-commit remote suites remain required.

Commands (serial execution on this small host):

```bash
CI=true ./scripts/pnpm.sh test
CI=true ./scripts/pnpm.sh --filter @loop-engine/providerd test test/cli.test.ts
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 just check
CI=true ./scripts/pnpm.sh build
```

The preceding cloud task `5594ea8` has all seven jobs passing in exact-commit
CI `35842723030`. That result does not substitute for this task's remote gates.
Fixture directories are deleted by teardown. Project-owned verification logs
are removed after recording outcomes. Unrelated files under `/tmp` are outside
the cleanup scope. Obsolete UV build environments and Rust intermediate object
files are reproducible caches, not research/audit records.

## Limits and rollback

No live API credentials or paid calls were used. Supplier profiles are implemented
and locally contract-tested, not `live_verified`. Optional model capabilities,
region entitlement, prices and input capacity require operator verification.
Unimplemented content/tools fail explicitly; they are not advertised as working.
Discovery/hot reload, compatible/self-hosted/gateway transports and platform
isolation/rate acceptance remain separate Phase 9 units.

The [configuration guide](../development/vendor-providers.md) documents plugin
IDs, secret references, regional origins, model-specific limitations and budgets.
Disable new routes before rollback, restore the previous binary/configuration
and regenerate resolution pins. Keep immutable invocation/continuation files,
including ambiguous calls. No destructive migration is necessary.
