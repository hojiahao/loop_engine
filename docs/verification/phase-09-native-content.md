# Phase 9 unit 2: Native streaming and rich messages

Status: implemented; local acceptance passed, publication/remote gates pending.

## Requirement and design

ADR 0039 extends the existing authenticated ProviderService for OpenAI Responses,
Chat Completions and Anthropic Messages. Unary and streaming share identity,
resolution/policy pins, input counting, token/cost/time bounds and the exclusive
invocation journal. Vendor codecs retain native content/usage semantics and never
execute tools. This task adds no research service, SQL table or Agent vendor branch.

Implemented paths include streamed text, interleaved function arguments and tool
results, registered structured output, private image/PDF prompt artifacts,
Responses/Claude reasoning continuation and separately priced Claude cache writes.
Optional features default off, are pinned to each configured model and are denied
when unavailable. Chat reasoning continuation is not advertised.

SSE transport and native state machines require complete lifecycle, final usage
and agreement with emitted previews before publishing a result. Exact completed
replay uses saved output; partial claims remain ambiguous without another supplier
request. Model/client schemas cannot become arbitrary executable validator input:
only administrator-registered, canonical, digest-pinned schemas enter strict Ajv.
Bounded JSON rejects duplicate keys, malformed Unicode, BOM and non-finite numbers.

Private prompt views are actor-scoped and accept no research/holdout path or URL.
Native signed/encrypted reasoning bytes remain in owner-only provider records;
the caller gets a reference bound to actor, provider, exact model, summary and
24-hour expiry. Cache-read and cache-creation usage are distinct subsets of total
input, with conservative reserve pricing and no fabricated billing receipt.

## Acceptance cases

The Provider suite contains 127 tests across eight files. In addition to unit 1's
52 tests, the rich-content cases exercise real TLS/gRPC and local HTTP/SSE fixtures:

- All three protocols: UTF-8 split across chunks, sequence numbering, complete
  text, two interleaved tool calls, tool-error continuation, strict registered JSON
  output, private image bytes and completed replay without another supplier call.
- Responses and Claude: reasoning summaries without wire disclosure of signatures
  or ciphertext, native continuation after host restart, changed summary rejection.
- Missing/duplicate completion, trailing events, Responses sequence gaps, final
  output disagreeing with previews, missing/over-budget usage and incomplete tool
  JSON all fail closed. Client cancellation is exercised on each protocol.
- Slow consumers do not cause the SSE wrapper to drain its source; cancellation
  releases that source. Chat's empty content chunk creates no phantom block, and
  tools arriving before text retain consistent content indices. Claude's terminal
  refusal retains `CONTENT_FILTER` classification.
- Unregistered/altered schemas, unmatched tool results, changed/symlinked prompt
  artifacts, research URIs, another actor's prompt namespace, cross-actor/model,
  expired and corrupted continuation references fail before outbound work.
- Claude cache-write/read totals, reserve pricing and rejection of unpriced
  one-hour retention. RFC 8785 numerical/property goldens and bounded JSON/schema
  negative cases preserve document integrity.

Protobuf adds `supports_documents`, optional cache-creation pricing and measured
cache-creation usage without renumbering existing fields or replacing the original
compatibility baseline. Generated Rust/TypeScript/Python bindings and the current
wire-fixture manifest are regenerated together. All three job validators accept
absent cache-write pricing for older snapshots and validate it when present.

## Verification commands

```bash
just check
./scripts/pnpm.sh test
./scripts/pnpm.sh build
./scripts/uv-protocol.sh run --locked --offline --no-sync pytest
CARGO_INCREMENTAL=0 ./scripts/cargo.sh test --locked --offline -p loop-protocol
```

The Provider test runner uses one file worker on the small development host;
independent 2/4/8-process claim tests are unchanged. Parallel file workers caused
CLI startup timeouts under host memory/I/O contention. The CLI test's outer
15-second bound covers its existing 10-second child deadline and cleanup;
application deadlines are unchanged. The final serial Provider run passed in 32.82s.
`just check` passes: 3,919 Python/Rust/Shell and 462 TypeScript/JavaScript function
naming checks, Protobuf generation/baseline/wire fixtures, Rust formatting and
all-targets/all-features Clippy with `-D warnings`, strict TypeScript, and Python
format/type checks. Clippy completes in 2m29s with cached dependencies. The Rust
protocol suite passes 27 tests; Python protocol passes 310 tests. The final
TypeScript workspace test run passes 119 protocol and 127 Provider tests. The Web
bootstrap has no behavior tests yet and is not counted as UI acceptance. All
TypeScript packages build successfully. Exact-commit remote CI remains the final
acceptance gate after publication.

The first Python protocol run caught a stale descriptor digest in the wire-fixture
manifest (309 passing cases, one failure); the manifest was regenerated from all
three pinned producers. This was not fixed by changing the immutable baseline.
pnpm's global-virtual-store choice is now explicitly disabled in workspace
configuration so host/CI detection cannot trigger an implicit store-mode reinstall.

Test fixtures remove their own `loop-provider-*` directories. Only this project's
rebuildable Rust incremental cache was removed during this task (about 383 MiB).
Private operational receipts and other projects' temporary directories are not
test cleanup targets.

## Limits and rollback

These are offline native contracts, not `live_verified` supplier access. No paid
LLM request, production database, market subscription or holdout is accessed.
Prompt image/PDF validation checks signatures and integrity, not full document
sanitization. Administrator schemas remain trusted configuration, constrained by
the documented supported profile. Provider OS/filesystem/egress isolation remains
unit 8, and durable cumulative run budgets remain Phase 10/11.

Continuation records expire for use after 24 hours but are not automatically
deleted; preserve records referenced by durable results. Their private receipt
encoding is not an at-rest encryption scheme. Native model support and exact
prices must be verified before enabling features; native token counters may fail
for a model and then block generation. Unsupported supplier features fail
explicitly rather than imply universal model availability.

Disable `PROVIDERD_DEPLOYMENT` to return to health-only operation. For a text-only
rollback, restore the prior executable/configuration and resolve that build's
pins. Preserve all invocation/continuation records and research/audit history.
No destructive down-migration or history rewrite is required. Configuration and
client steps are in `docs/development/native-providers.md`.
