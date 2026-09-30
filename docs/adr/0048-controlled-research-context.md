# ADR 0048: Controlled research tools and durable context

Status: accepted and implemented; Phase 10 unit 3 complete. Commits `b33f0f5`
and `8199a78` are pushed; exact-commit CI `36590096683` passed all seven jobs.

## Requirement

A model may request a registered research operation and continue from its actual
result. Neither tool arguments nor resumed clients may replace authority, frozen
data, conversation history or cumulative budgets. A crash must not duplicate a
paid model invocation. Existing one-step plans and immutable evidence remain
readable.

## Decision

Add a closed `loop.discovery-plan/v2` profile: exactly two model calls with one
read-only `research_describe` call between them, producing at most one final AST
candidate. Pin both invocation templates. The initial messages and model/policy
identity must agree; later assistant/tool messages come only from durable
receipts. The first invocation requires the registered tool; the final invocation
disables further tool generation and requires the existing AST schema. This is
not an autonomous research loop or a general executable tool host.

The first registered tool accepts only `{}`. Its target is the plan's existing
development dataset and installed operator registry. Reuse the authenticated
Discovery job boundary and actual `manifests::data::resolve(..., false)` file
verification. Return a bounded, allowlisted description of the verified sample,
snapshot identities, artifact schemas and installed fields. Do not return paths,
raw market data, secrets or capabilities. Unknown tools, arguments, protected
samples and changed files fail closed. This is descriptive evidence, not factor
evaluation, current portfolio metrics, profitability or admission. Numerical
operations continue through the separate authorized Python research workflow.

Reuse typed `ModelMessage`, `ToolCallContent` and `ToolResultContent`. Full model
requests already hold the exact sent conversation; do not add a duplicate context
snapshot table. Rebuild the second request from the frozen initial messages,
persisted assistant call and matching committed tool result. Bound message count,
serialized context size and call/result count before reserving or sending.

Migration 0012 extends model steps with an immutable ordinal and adds append-only
tool-result evidence. Old rows keep ordinal zero and all original request,
response, digest and budget bytes. Every update and replay names its ordinal;
historical steps cannot be mistaken for the latest call. Intermediate model
completion preserves the running job and fenced lease. Final candidate completion
retains the existing terminal outcome path.

Reserve each invocation transactionally against the sum of all prior input,
output and exact USD ceilings, including ambiguous calls. Enforce the three-step
job limit and original absolute deadline. No implicit refund or new job budget is
created. Dispatch remains CAS-only; every previously dispatched call is recovered
through Provider lookup. The read-only tool may repeat verification if it crashed
before result commit, but its committed result is immutable and idempotent. Do not
claim physically exactly-once file reads.

## Acceptance and rollback

Require real PostgreSQL and mTLS Provider execution across both model turns and
the verified tool; exact typed-context/digest goldens; denied tools, altered
history, oversized context, budget excess and protected-data tests; independent
2/4/8 writers; and crash cuts around tool/result/next-dispatch commits. Retain the
single-step regression suite. Errors remain operational failures, not empirical
factor rejection. No paid supplier or production research is required for these
synthetic acceptance cases.

Stop old writers before migration. Fence pre-migration model writers explicitly.
Rollback disables v2 plan submission/execution on a migration-aware binary and
preserves ordinals, tool evidence, model reservations, receipts and audit. Do not
rewrite migration 0011 or delete histories. Commit and push this complete task
with its documentation and evidence; exact-commit remote CI is the final gate.
