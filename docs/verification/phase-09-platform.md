# Phase 9 unit 8: Provider platform acceptance

Status: implementation, complete TypeScript regression, targeted Rust process
tests, `just check`, workspace TypeScript build and combined actual-container
isolation gates passed on 2026-09-24. Commit, push and exact-commit CI remain
required. Phase 9 remains in progress until those gates succeed.

## Requirement, scope and evidence

ADR 0045 completes the existing service with bounded process-level admission,
integer price accounting and a real isolated runtime. No research database
table or Provider-specific logic in the Loop/research worker is added. A small
infrastructure CONNECT gateway provides a separately enforced outbound boundary;
it does not route models, decrypt supplier TLS or hold API keys.

Observed on 2026-09-24:

- Ten new Provider cases passed for sliding-window request/token/cost admission,
  exact-window recovery, monotonic clock regression, concurrent callers,
  pre-claim denial, upstream failure retention and exact cache/rounding prices.
- Six gateway cases passed with actual TCP tunnels, exact destination
  matching, private-address denial, no credentials, no wildcard and no plaintext
  forwarding. Rejected half-open/error connections are destroyed, and payload
  arriving during DNS/TCP setup is buffered without data loss.
- Two dependency-boundary cases passed against the actual Provider, Rust
  orchestration and Python research sources, including import-parser fixtures.
- The complete TypeScript run passed 686 Provider cases across 22 files and
  119 protocol cases; the locked TypeScript workspace build passed.
- `just check` passed with Rust fmt and Clippy `-D warnings`, schema/wire
  checks, TypeScript formatting/lint/types and all four Python environment
  checks. Naming gates cover 3,921 Python/Rust/Shell and 686 TypeScript/JavaScript
  declarations. No quality check was weakened or disabled.
- The combined `just test-isolation` gate passed both actual containers: the
  original research-role boundary (6.42 seconds) and Provider (20.76 seconds).
  The latter completes an invocation using the compiled Provider, independent
  mTLS client, approved tunnel and synthetic HTTPS supplier. Known protected
  paths and research/source trees are absent, root
  writes fail, capabilities are dropped, UID is 65532, direct supplier-network
  access and external DNS fail, and unapproved tunnels return 403.

The preceding catalog commit's CI `35965098459` passed six jobs, including Rust
and unified workspace gates, but the DaoCloud container failed the existing
`cancelled_descendant_dies` test. Review reproduced a readiness race capable of
causing that assertion: Python `write_text` creates the PID
file before writing its contents; checking only file existence could read an
empty PID and mistakenly inspect the valid system file `/proc//stat`. The test
now deliberately starts with an empty file, waits for a strict positive PID and
publishes the complete PID using atomic rename. Linux process state is parsed
after the parenthesized command name, which may itself contain spaces or `)`.
All six targeted real-process Rust cases pass. Production SIGKILL behavior and
the five-second assertion deadline are unchanged. A direct attempt to run the
host binary in the older container runtime failed at the glibc loader; it is
not counted as container evidence. The corrected clean build must pass CI.

The initial host-port fixture failed because Docker's internal network exposed
no published host port despite the requested binding. The service was healthy
and listening inside its namespace. The delivered profile explicitly uses the
internal network and an independent control-plane client; no external network
was added to the Provider to make the test pass. The final runtime policy and
fixture agree on this deployment boundary.

Existing per-call budgets, zero generation retries, redacted typed errors,
immutable replay, ambiguous-call fencing and 2/4/8-process journal/catalog tests
remain mandatory. The local window is not a persistent multi-instance budget;
Phase 10 retains that responsibility. Price estimates and reserved non-token
fees are not reported as measured supplier charges.

README is now product-oriented: verified installation, actual CLI/API entry
points, data and model prerequisites, research/independent workflows and recovery.
It explicitly distinguishes the delivered services from the pending autonomous
Run Harness/Loop and operational Web/TUI. Phase progress and verification history
remain in maintenance documents. README local links and `git diff --check` pass.

## Combined contract matrix

| Group | Implemented routes | Contract evidence |
| --- | --- | --- |
| Native | OpenAI Responses/Chat, Anthropic Messages | Invocation, rich content, streaming, private continuation and journal suites |
| Additional native | Google GenerateContent/Interactions, Cohere V2 Chat | Additional protocol and state suites |
| Cloud deployments | Azure Responses/Chat, Vertex GenerateContent, Bedrock Converse | Cloud identity, signed requests, binary streams and state suites |
| Vendor plugins | Mistral, DeepSeek, Qwen, xAI, Groq, Together, Fireworks, Cerebras, Perplexity, GLM, Kimi, MiniMax | Vendor and state suites |
| Compatibility/gateways | OpenAI-compatible Chat/Responses, Anthropic-compatible, Ollama, vLLM, SGLang, llama.cpp, LM Studio, NIM, LiteLLM, Portkey, OpenRouter | Compatible transport and state suites |
| Catalog | Official lists, pinned signed sources, local overrides, immutable publication, hot activation | Discovery, catalog, process and compiled CLI suites |
| Platform | Request windows, price arithmetic, actual process/data/network isolation | Platform, dependency-boundary, egress and container suites |

Fixtures cover supported tool, JSON, media, thinking/cache, stream, cancellation,
timeout, usage, malformed response, identity and no-retry paths. They are not
live model verification. No credentials for paid models were supplied to these
gates, and unsupported model-specific capabilities stay denied.

## Reproduction and rollback

```bash
CI=true ./scripts/pnpm.sh test
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CI=true just check
CI=true ./scripts/pnpm.sh build
just test-isolation
```

The TypeScript CI job and unified/container gates run the actual Provider
isolation test after building its assets. The original research-role isolation
test remains intact. Runtime fixtures clean only their own unique temporary
paths, containers and networks; no production data or other project's `/tmp`
files are touched.

See [runtime deployment](../development/provider-runtime.md) for exact paths,
UID/modes, approved destinations, cloud identity and network restrictions,
operator commands and rollback. Stop writers/listeners before restoring a prior
binary/configuration, and preserve all immutable catalog and invocation state.
