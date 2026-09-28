# Phase 9 unit 1: Authenticated native text invocation

Status: accepted. Implementation `fc0068073a37fb18b9468bd9b252cea0034e8c91`
is pushed; GitHub Actions run `35701264148` passed all seven jobs, including
Rust, unified workspace and the clean DaoCloud development container.

## Requirement and design

ADR 0038 connects the existing ProviderService to actual native OpenAI Responses,
OpenAI Chat Completions and Anthropic Messages transports. The installed Node
executable now has an optional TLS 1.3 gRPC listener; health alone cannot invoke
a model. Official SDKs are pinned to OpenAI 7.20.0 and Anthropic 0.127.0, with
automatic retries disabled. Connect uses the existing generated service schema.
No provider branch enters the Rust orchestrator or Python research worker.

Deployment-owned certificate fingerprints authenticate callers before Actor
metadata is checked. Exact model and request-policy snapshots, token/cost/time
budgets, native token preflight and bounded responses gate every invocation.
Endpoints are fixed official origins and redirects are denied. Errors contain
typed allowlisted codes, not native SDK messages or secrets. Usage is normalized;
calculated price reserves are not misrepresented as supplier billing receipts.

A separate private journal claims an actor-scoped idempotency key before any
supplier request and exclusively publishes a checksummed completed response.
An uncertain interrupted attempt remains fenced on restart; a completed exact
retry replays without another supplier call. This deliberately sacrifices
automatic retry availability rather than risk duplicate external charges. It
does not claim exactly-once API execution or replace later run-wide budgets.

## Local acceptance

The native suite passes 52 tests across five files. Evidence exercises actual
HTTP supplier fixtures, real TLS/gRPC connections and the compiled executable:

- Exact native request shapes and credentials for all three paths, normalized
  total/cached input usage, explicit refusals and length termination.
- Missing/unregistered client certificates, forged actors, forwarded or holdout
  metadata, stale timestamps and absent RPC deadlines.
- Model/capability/policy drift, unsupported content and streaming, missing
  credentials, money/token limits, concurrent invocation limits and clock rollback.
- 429 without automatic retry, SDK error redaction, redirects, oversized output,
  invalid variants, absent usage and final reported token overrun.
- Cancellation after the supplier observes the request and operation deadlines;
  uncertain retries cannot start another request. The cancellation case is
  synchronized with request arrival, not a race against an arbitrary delay.
- Exact replay after host restart, 2/4/8 independent OS claim writers and real
  kill/restart before and after result publication. Corrupt results fail closed;
  a later result cannot overwrite an existing one.
- Journal parent/directory durability before dispatch; private-file modes and
  symlink refusal; exact integer money and
  canonical JSON validation; source/dependency model pins; actual `--describe`,
  redacted startup failure, health, authenticated RPC and SIGTERM shutdown.

Final local commands pass:

```bash
just check
./scripts/pnpm.sh check
./scripts/pnpm.sh test
./scripts/pnpm.sh build
```

`just check` passes the Protobuf/schema and cross-language gates, 3,917
Python/Rust/Shell and 406 TypeScript/JavaScript naming checks, Rust formatting,
all-targets/all-features Clippy with `-D warnings`, strict TypeScript and Python
style/type checks. Clippy completes in 10m10s on the small development host.
Provider tests are also included in strict TypeScript checking; this caught and
corrected a test using a unary request object as a streaming request.

The TypeScript gates were run after deleting the two verified, untracked Provider
and protocol `dist` directories (about 1 MiB), proving the package scripts build
their required bindings from an empty output directory. All 115 protocol tests
and 52 Provider tests pass, followed by all TypeScript package builds. The Web
bootstrap has no behavioral test files yet and is not counted as UI acceptance.
The full Rust/research behavioral suites also passed in exact-commit remote CI;
the local native fixture count does not substitute for those gates.

All supplier calls in these tests go to loopback fixtures. No paid model API,
production database, licensed market data or holdout is accessed. Test-owned
`loop-provider-*` temporary directories are removed by their fixtures; private
operational invocation journals are not disposable test artifacts.

## Limits and remaining work

This task delivers unary text only. Phase 9 unit 2 must implement streaming,
tools, structured output, reasoning continuation, prompt caching and prompt-safe
artifact access before those capabilities can be advertised. Other native/cloud
protocols, vendor plugins, catalog hot reload and complete provider process/egress
isolation remain later units. Health and offline contracts do not establish
`live_verified` status. No real supplier credentials were exercised here.

## Rollback

Unset `PROVIDERD_DEPLOYMENT` and restart to restore health-only behavior. Keep
all private claims/results, configuration snapshots and research/audit history;
never remove an ambiguous claim merely to permit another charge. This task has
no SQL migration or production-state change. An older executable can run health
but cannot serve the new optional native RPC workflow. Operational steps are in
`docs/development/native-providers.md`.
