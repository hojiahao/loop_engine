# Phase 9 unit 3: Google and Cohere native protocols

Status: implemented; local check/test/build passed on 2026-09-23. Chinese task
commit `d5d49de` is pushed. All seven jobs in exact-commit CI `35830578199` passed.
This does not close Phase 9 or authorize a merge to `main`.

## Requirement and implementation

ADR 0040 connects Google GenerateContent, Google Interactions and Cohere V2 Chat
to the existing authenticated ProviderService. Each route has its own native
request, response and stream translation. Official SDKs are pinned to
`@google/genai` 2.24.0 and `cohere-ai` 8.1.0. Both generation and streaming use
those SDKs against fixed supplier origins, with retries disabled and the shared
bounded transport. No new service, research table or Loop vendor branch is added.

All routes reuse mTLS principal authorization, pinned model/policy resolution,
registered schemas, private prompt artifacts, private reasoning continuation,
token/cost/time bounds and the durable invocation journal. A successful stream
requires complete native lifecycle, consistent previews, final usage and a
published receipt. Cancellation or ambiguous delivery cannot silently retry a
potentially charged request. The Provider returns function proposals, never
executes tools or receives a research/holdout capability.

GenerateContent's token counter uses bounded native HTTP with the documented
`generateContentRequest` envelope; the installed SDK's Developer API count method
cannot retain the full system/tool configuration. Interactions and Cohere have
no full chat-envelope counter in this profile. Their configured
`input_token_limit` must be the documented vendor maximum, and the caller must
reserve that full ceiling before dispatch. This is conservative reservation, not
a measured prompt count. Final usage must also fit the combined context limit.

Google signed Parts and Interactions thought steps stay in private continuation
records. Gemini continuation compares the visible JSON meaning and replays the
original signed bytes; local IDs for legacy ID-less calls are never sent as native
IDs. Consecutive tool-result messages become one Gemini user turn. Cohere retains
thinking content privately and treats tool plans as ordinary assistant text.
Actual Cohere `usage.tokens` is separate from `billed_units`; no billing difference
is inferred to be a cache hit. All prices and capabilities remain administrator
declarations until catalog and live-verification work supplies further evidence.

## Executable acceptance

The two additional test files contain 60 cases using actual TLS/gRPC listeners,
the installed official SDKs and local HTTP/SSE supplier fixtures:

- All three routes: native authentication and request paths, client tool/result
  round trips, registered structured output, incremental text/tool streams,
  completed receipt replay and reasoning continuation.
- Verified private image bytes on all routes and PDF bytes on both Google routes;
  Cohere PDF capability is denied at deployment.
- Missing usage, exceeded caller budgets, Google model drift, redacted HTTP 429,
  truncated/trailing streams and cancellation fail without a successful receipt.
  SDK retries remain disabled, and ambiguous resubmission sends no new request.
- Interactions/Cohere input ceilings are required, fingerprinted and bounded by
  configured context. Insufficient input reservation denies work before dispatch.
  Unsupported Cohere named tool selection is also denied before transport.
- Google signed function continuation rejects a changed visible tool. A client's
  equivalent JSON whitespace normalization preserves continuation while the
  original native bytes are replayed. Two parallel tool results are grouped in
  one user turn with their original function names and IDs.
- Google/Cohere streamed thinking uses private continuation references; Google
  signatures never appear in RPC content. Google thought usage is a measured
  output subset. Cohere does not fabricate a separately measured thinking count.
- Unsupported Interactions citations and malformed thought summaries fail in
  unary responses and before stream previews. Cohere thinking followed by a tool
  plan preserves all three reasoning/text/function blocks and their ordering.

Existing tests retain executable startup, authentication, schema/artifact denials,
2/4/8 independent journal writers, kill/restart, cancellation, clock regression
and duplicate-spend prevention. Local fixture success is offline contract evidence,
not evidence of a real account's model access or supplier billing.

Verification commands:

```bash
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 just check
./scripts/pnpm.sh test
./scripts/pnpm.sh build
```

The final TypeScript workspace test command passes all 187 Provider cases across
10 files and 119 protocol cases across 13 files. The Provider command, including
its compilation steps, completes in 58.1s on this host. `just check` passes 3,919
Python/Rust/Shell and 499 TypeScript/JavaScript function-name declarations,
Protobuf generation and compatibility/three-language fixtures, Rust formatting
and all-targets/all-features Clippy with `-D warnings`, TypeScript format/lint/type
checks, and Python format/type checks. Clippy completes in 49.34s with the existing
cache and one build job. TypeScript protocol, Provider and Web builds all pass.
The Web bootstrap has no behavior tests and is not counted as UI acceptance.

Run the checks and Provider suite sequentially on the small development host.
Overlapping Clippy and the CLI integration suite caused heavy paging/I/O pressure
and a startup timeout; that overlapping run was stopped. The final serial runs
pass without extending the application's or tests' deadlines. These timings are
acceptance observations, not performance guarantees.

The first complete Provider run exposed three existing CLI startup timeouts when
unused large SDKs were eagerly imported. Loading Google/Cohere SDKs only for
configured routes with available credentials fixed the regression; the original
child-process startup deadline was not increased. The HTTP fixture also now
preserves a supplied native `Request`'s body, headers and cancellation signal when
redirecting it to the local test listener. These fixes exercise the real installed
SDK transport rather than replacing it with a mocked SDK method.

Fixture directories use the project's `loop-provider-*` prefix and are removed
by test teardown. No live key, paid LLM request, production database or holdout is
used. Other projects' temporary files are not cleanup targets.

## Limits and rollback

The supported research profile is text, client functions, registered JSON output,
verified prompt images/PDFs and native thinking as documented in
`docs/development/native-providers.md`. Hosted tools, grounding, audio/video,
generated media, image thought summaries and supplier-managed/background
conversations remain unavailable. Unsupported output fails closed.

Cohere named tool choice, mixed per-tool strictness and structured output with
tools have no implemented lossless representation and are rejected. Its API does
not echo the resolved model ID: the receipt binds the requested versioned ID but
cannot independently prove a supplier revision. Google requires the native model
echo to match the pinned bare ID. No route is `live_verified` here.

Cloud deployments, other suppliers, compatible/gateway transports, hot reload,
official discovery and actual Provider process isolation remain later Phase 9
units. Durable cumulative run budgets remain Phase 10/11. These exclusions are
not presented as completed capabilities.

Remove the three new routes from private deployment configuration to disable new
work. To roll back the code, restore the previous executable and its configuration
and regenerate that build's resolution pins. Preserve invocation claims/results
and continuation records; do not delete them to retry ambiguous requests. No SQL
migration, destructive down-migration or research-history rewrite is needed.
