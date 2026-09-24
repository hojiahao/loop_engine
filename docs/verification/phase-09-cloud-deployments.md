# Phase 9 unit 4: Cloud deployments

Status: accepted; local check/test/build passed on 2026-09-23. Task commit
`5594ea8` is pushed; exact-commit CI `35842723030` passed all seven jobs.
Phase 9 is not complete; `main` is not merged.

## Requirement and implementation

ADR 0041 connects `azure_responses`, `azure_chat`, `vertex_generate` and
`bedrock_converse` to the existing authenticated ProviderService. Azure and
Vertex reuse the native OpenAI and Gemini content/stream codecs. Bedrock owns
its Converse message translation and binary EventStream validation. No Rust,
Python, research state, SQL schema or Agent-loop vendor branch changes.

Azure v1 sends the deployment name while retaining resolved-model checks on
replies. Key and Entra identity modes are mutually exclusive. Vertex binds the
public-cloud project, location and Google publisher, using ADC/OAuth and a native
model-version check. Bedrock uses official SDK SigV4 with explicit credentials
or the default chain, pins region/model selector and optional published Guardrail,
and preserves provider-specific reasoning and cache semantics. Bedrock has no
model-version echo; its backend mapping remains an administrator declaration.

Cloud identity lookup shares the operation deadline, with no delayed dispatch
after cancellation. Retries and redirects are disabled for generation. Cloud
routes reserve the full declared input capacity rather than using an incomplete
counter. Guardrails require a separately declared worst-case per-call USD bound,
added before generation; token pricing does not implicitly cover that service.
Actual usage is independently validated, and no supplier invoice is fabricated.

The additional direct dependencies are exact-pinned official SDK components:
`@aws-sdk/client-bedrock-runtime` 3.1136.0,
`@aws-sdk/credential-provider-node` 3.972.83, `@azure/identity` 4.13.3,
`@smithy/core` 3.34.1 and `google-auth-library` 10.9.1. The newer AWS release did
not meet the existing 24-hour release-age policy; an eligible version was used
without bypassing that policy. Existing Google/OpenAI SDK versions are unchanged.

## Executable cases

The cloud test files exercise installed SDKs through actual TLS/gRPC and local
HTTP vendor fixtures. They cover:

- All four routes: unary text, tool/result round trips, registered native schema
  output, authorized image/PDF transport, complete streaming receipts, full-input
  reservation, usage validation, redacted HTTP 429 and redirect refusal.
- Independent HMAC recomputation over the received AWS request body, canonical
  path, signed headers, date, region and session credential scope. The local
  fixture's Host rewrite is separate from the pinned signed AWS destination.
- Azure key versus Bearer headers, distinct deployment/resolved model selectors,
  Vertex regional/global origins and project paths, OAuth failure/deadlines and
  missing explicit AWS credentials without identity fallback.
- Default AWS credential-chain selection with an ambient Bedrock bearer token:
  generation still uses SigV4. Cloud configuration and routing changes alter
  model resolution pins. Prompt-router selectors and unsupported partitions fail.
- Bedrock signed reasoning/tool continuation after constructing a fresh Host,
  original streamed tool JSON bytes, five-minute cache reads/writes and a
  separately budgeted published Guardrail.
- Missing usage, oversized/truncated frames, corrupted CRC, unknown events,
  unknown content unions, duplicate terminal events, stream exceptions,
  cancellation and ambiguous replay. None may publish a successful receipt.

The installed SDK silently skips unknown binary event names. The transport
allowlists event names and output unions before SDK projection, in addition to
CRC, length and JSON bounds. Negative cases prevent unsupported content from
disappearing from a successful receipt. The SDK's JSON request adapter is a
Uint8Array subclass: transport preserves those signed bytes without coercion or
re-serialization. Independent HMAC verification covers that boundary.

Existing native Provider tests remain part of the suite, including the compiled
CLI, mTLS authentication, schema/artifact isolation, 2/4/8 independent journal
writers, kill/restart, clock regression and duplicate-spend prevention.

## Verification commands

Run serially on this small host:

```bash
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 just check
CI=true ./scripts/pnpm.sh test
CI=true ./scripts/pnpm.sh build
```

`just check` passes on the final implementation: 3,919 Python/Rust/Shell and 545
TypeScript/JavaScript function declarations meet the naming contract; Protobuf
generation/compatibility and cross-language fixtures match; Rust fmt and Clippy
with all targets/features and `-D warnings`, TypeScript format/lint/type checks,
and the research/protocol/independent-worker Python static checks pass. The
initial sandbox attempt could not spawn the Git subprocess used by the naming
check; the authorized rerun completed successfully without changing that gate.

The final TypeScript workspace suite passes all 269 Provider tests in 12 files
and all 119 protocol tests in 13 files. The Provider runner takes 63.61 seconds
on this host, excluding its preceding compilation; 82 cases exercise the new
cloud paths. The actual compiled CLI, journal concurrency and prior native
protocol tests remain green. This observation is not a performance guarantee.

TypeScript protocol, Provider and Web builds all pass. The Web bootstrap has no
behavior tests and is not counted as UI acceptance. No Rust or Python behavior
changed in this unit; their remote workspace/container suites passed for
published commit `5594ea8` in run `35842723030`. The preceding Google/Cohere commit `d5d49de` has all
seven jobs passing in exact-commit CI `35830578199`; that result is not reused as
CI evidence for this new commit.

Test fixtures remove their project-owned temporary directories in teardown.
Temporary verification logs are removed after recording their outcome; other
projects' files under `/tmp` are not cleanup targets.

## Limits and rollback

OAuth contracts use injected local token resolvers. They verify headers,
deadlines, cancellation and redaction, not a real Entra/ADC login, token refresh,
IAM grant or account entitlement. There are no live cloud keys or paid requests
in this evidence. These adapters are not `live_verified`. Cloud private endpoint
configuration, sovereign partitions, Vertex partner protocols, Bedrock hosted
tools/prompt management and other reasoning dialects are not silently advertised.

The [cloud configuration guide](../development/cloud-providers.md) specifies
credential injection, model/deployment mapping, per-call reservations, supported
native capabilities and operational limits. Catalog discovery/hot reload,
additional vendors/gateways and process isolation/platform acceptance remain
later Phase 9 units. Cumulative run budgets remain Phase 10/11.

To roll back, disable the new routes, restore the prior binary's configuration
and regenerate its pins. Preserve invocation claims/results and continuation
records, especially ambiguous calls. No database migration, historical rewrite
or research/holdout authority change is involved.
