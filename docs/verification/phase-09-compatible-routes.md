# Phase 9 unit 6: Compatible, self-hosted and gateway routes

Status: implementation, 172 local route cases, workspace regression, quality
gates and build passed on 2026-09-24. Chinese task commit `9d197c7` is pushed;
all seven jobs in exact-commit CI `35958808670` passed. Phase 9 is not complete;
`main` remains unchanged.

## Requirement, design and tradeoffs

ADR 0043 connects OpenAI-compatible Chat/Responses, Anthropic-compatible
Messages, Ollama, vLLM, SGLang, llama.cpp, LM Studio, NVIDIA NIM, LiteLLM,
Portkey and OpenRouter to the existing TLS/gRPC invocation workflow. No service,
database table, model runtime, SDK dependency or Rust/Python routing branch is
added. Installed SDKs and existing native stream/schema/continuation validation
remain shared. The compatible dialect extends the validated vendor reply
normalizer; existing vendor cases must therefore remain green.

Private administrative configuration fixes endpoints, credential modes, model
selectors, implemented capabilities and dialects. Requests cannot introduce
URLs. Plain HTTP is restricted to literal loopback; redirects are rejected.
Anonymous endpoints receive no fabricated authentication. Every request reserves
the declared maximum input rather than using an unverified tokenizer.

Gateway identity remains separate from declared upstream identity. The catalog
digest binds the reviewed gateway mapping; runtime controls disable supported
automatic fallback/retry paths. Declared routing hashes are not independent
remote attestations. Returned-model checks and optional Portkey retry receipts
can detect contradictions, but live deployment verification is still required.
Gateway non-token ceilings participate in pre-dispatch reservation. Unreported
fees are not invented as measured invoice charges.

Generic Chat supports explicit plaintext reasoning fields and preserves private
continuation across restarts. Native-compatible Responses/Messages retain their
existing encrypted/signed state handling. Unsupported opaque Chat reasoning and
strict tools fail explicitly; a generic compatible label does not establish
every capability of every model or serving version.

## Executable acceptance

`compatible.test.ts` and `compatible-state.test.ts` contain 172 cases using actual
SDK requests, a local HTTP upstream and authenticated TLS/gRPC calls:

- Twelve wire routes: unary text, ordered text/function streams, tool result
  turns, registered JSON-schema output, exact credential headers, missing usage,
  redirects and redacted 429 errors with no generation retry.
- Eleven thinking routes: unary/streamed private continuation after a fresh Host,
  exact deployment selector versus returned identity, native private signature
  recovery, tampered-summary rejection, terminal/separate usage dialects.
- Every route: truncated streams and substituted response models cannot publish
  success. Three native wire families: cancellation leaves an ambiguous claim,
  and replay cannot submit another generation request.
- Anonymous Ollama receives neither Authorization nor an API-key header. Portkey
  requires two independently validated secrets and rejects contradictory retry
  receipts. Unknown strict requirements are denied before dispatch.
- All three gateways reserve non-token fees before dispatch; mapping changes
  change resolution pins. Dynamic OpenRouter selectors, unsupported opaque
  reasoning, unsafe URLs and Chat-only options on other wires are rejected.

Observed targeted run on 2026-09-24: 2 files, **172 passed**, 16.04 seconds.
Type checking passed before this run. The subsequent complete TypeScript run
passed **594 Provider cases** across 16 files (88.15 seconds), including the
existing native/cloud/vendor workflows, and **119 protocol cases**. The Web
bootstrap has no behavioral tests and is not UI acceptance.

The first `just check` invocation hit the execution sandbox's `spawnSync git
EPERM` restriction in the naming checker. The unchanged command passed when
rerun with the required process permissions; no assertion or quality gate was
disabled. All 3,919 Python/Rust/Shell and 618 TypeScript function declarations
pass the naming rule. Protocol generation, cross-language fixtures, Rust fmt
and Clippy (`-D warnings`), TypeScript format/lint/types, and all four Python
environment checks passed. Protocol, Provider and Web workspace builds passed.
The preceding vendor task `e1055c6` has all seven jobs green in CI `35953153780`;
that result is separate from this task's remote acceptance.

```bash
./scripts/pnpm.sh --filter @loop-engine/providerd typecheck
./scripts/pnpm.sh --filter @loop-engine/providerd exec vitest run test/compatible.test.ts test/compatible-state.test.ts --maxWorkers=1 --no-file-parallelism
CI=true ./scripts/pnpm.sh test
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CI=true just check
CI=true ./scripts/pnpm.sh build
```

No live credentials, paid generation or model-weight download was used.
All fixture directories are removed by teardown. Project-owned temporary logs
are removed after recording outcomes; other projects' `/tmp` assets are untouched.

## Operations, remaining gates and rollback

The [deployment guide](../development/compatible-providers.md) documents exact
plugin IDs, authentication, paths, dialect settings, gateway controls and limits.
Deployment verification must establish server version, model identity, enabled
features, prices, capacities and route stability before research traffic.

Catalog discovery/hot reload and platform isolation/rate acceptance remain the
next Phase 9 units. These fixture results do not establish `live_verified`.
Disable new routes before restoring the preceding binary/configuration;
regenerate model pins and preserve immutable invocation and private-state files.
No destructive schema migration or audit rewrite is introduced.
