# Controlled research tools and persistent context

The optional `loop.discovery-plan/v2` profile runs a fixed conversation:

1. The model requests the registered `research_describe` tool.
2. The runtime verifies the plan's actual development files and commits a bounded
   description of the sample, snapshots, artifact schemas and installed fields.
3. The model receives that exact saved result and returns one canonical AST.

This tool describes authorized inputs. It does not calculate factors or portfolio
metrics, grant research/holdout permissions, execute code, or admit a candidate.
The numerical workflows remain separate. Existing `loop.discovery-plan/v1`
plans keep their original single-call behavior.

## Prepare a v2 plan

Use the private mTLS deployment, content-addressed plan store and same registered
Discovery identity described in [the single-step guide](durable-model-step.md).
Stop old model writers before applying migration `0012_tool_context.sql` with the
deployment migration identity. The migration preserves every original request,
response, checksum and reservation; it adds ordinal zero to previous calls.

Generate and execute the existing deployment bundle with
`node scripts/postgres-production-bundle.mjs`, following the administrative
procedure in [PostgreSQL deployment](postgresql.md). Applying the migration SQL
alone does not refresh runtime privileges. The bundle grants the application
role `UPDATE` on the explicit mutable-table list, including `model_steps`;
`tool_results` receives only `SELECT` and `INSERT` and remains immutable.

Register these three immutable schemas in the Provider deployment:

- `config/schemas/discovery-ast.v1.json`: final AST output.
- `config/schemas/research-describe.v1.json`: closed empty tool arguments.
- `config/schemas/research-description.v1.json`: the typed tool result.

Convert each formatted source to compact recursively key-sorted JSON before
hashing. Use its exact schema ID, version 1 and SHA-256. The schema goldens bind
these deployable documents to the runtime's definitions. Provider schema
registration is required for tool-result JSON even though the model does not
choose that schema. Do not broaden the accepted schema vocabulary.

Resolve an explicit model supporting tools and structured output. Its frozen
capabilities must permit both. This profile does not transport reasoning state;
use a route without reasoning continuation. Provider-specific restrictions still
apply: a capability flag does not waive a supplier's request validation.
For an OpenAI-compatible route, `compatible.strict_tools` must explicitly be
`true` and the endpoint must actually support strict tool schemas. A general
tools capability does not enable this separate transport requirement.

Pin two `ModelInvocation` protobuf templates:

| Field | `tool_invocation` (first call) | `invocation` (final call) |
| --- | --- | --- |
| Model and request policy | Exact Provider snapshot/policy | Same values |
| Messages | System/user text, no previous calls | Exactly the same initial messages |
| Request ID | Absent | Absent |
| Tools | Sole strict `research_describe` definition | Identical definition, retained for history validation |
| Tool choice | REQUIRED, or NAMED `research_describe` | NONE |
| Structured output | Absent | Existing strict AST schema |
| Budget | Explicit positive per-call ceilings | Explicit positive per-call ceilings |

The registered tool description is exactly:

```text
Describe the frozen development research inputs and operator registry.
```

Its schema ID is `loop.research-describe/v1`. It accepts only canonical `{}`;
there is no caller-selected dataset, URI, path, SQL, command or capability.
Initial messages should tell the model to inspect the tool result and then
return the required AST. Treat the result as data, not instructions.
The frozen initial context is limited to 30 messages and 110 KiB, leaving space
for the mandatory assistant/result pair before any paid request is reserved.

Set the Discovery job budget to three steps and one maximum candidate. Its input,
output and USD ceilings must cover the **sum** of both templates. Allow 35 seconds
in addition to both invocation wall budgets for bounded tool verification and
commit overhead; their sum must be strictly below the original job wall budget
(at most 120 seconds). For example, two five-second calls fit inside a
120-second job. These are ceilings, not actual supplier charges.

Use the same immutable plan document as v1, with:

```json
{
  "schema": "loop.discovery-plan/v2",
  "tool_invocation": {"sha256": "sha256:<first template digest>", "byte_size": 123},
  "invocation": {"sha256": "sha256:<final template digest>", "byte_size": 123}
}
```

The fragment above replaces/adds only those fields; all owner, run, connector,
input, registry, data and protocol references from the complete v1 document are
still required. Add `discovery.tool-context.v1` to the protocol feature list,
sort it and recompute the selection digest. Publish objects before the final
plan, then pin the plan's digest in runtime configuration. A v1 plan cannot opt
into tools by adding an extra field.

## Execute and inspect

Use the existing authenticated `StartDiscovery`, `ExecuteDiscovery` and
`GetDiscovery` RPCs with fresh contexts and explicit deadlines. There is no
caller conversation upload and no new generic tool RPC. One Execute request can
finish both calls and the tool if its deadline permits. Use the returned job
revision for a subsequent command.

The response projects the latest model-step state and **cumulative job** token
and USD reservations. `COMPLETED` with a `RUNNING` job means the current call has
a saved response but the conversation is not finished. A candidate appears only
after terminal success. Inspect job status as well as step state.

The private durable history contains ordinal zero and one model requests, the
original assistant call, the tool result and audit evidence. The final request
is reconstructed as frozen initial messages, saved assistant call, and saved
tool result. It is bounded to 32 messages and 128 KiB of encoded context; the
description itself is at most 16 KiB. Exceeding a limit fails before another
model dispatch. No caller can insert, delete or reorder historical messages.

## Resume and rollback

Every model request is reserved and dispatched independently with a new original
request identity. Once dispatched, that particular request can only be looked up;
missing evidence never permits a paid resend. Every reservation stays in the
cumulative job total, including uncertain calls.

If interruption occurs after the first model receipt but before the tool result
commit, the read-only tool may repeat file verification under renewed authority.
Once a result is committed, continuation uses its immutable bytes. This is
idempotent result publication, not a claim of physically exactly-once reads.
Unregistered tools, changed files or unavailable evidence remain operational
failures and cannot create empirical factor rejections or admissions.

Recovery observes the existing revision, lease and absolute deadline. A live
worker's lease cannot be borrowed by another Execute request. A committed
intermediate response renews the bounded tool lease; resumption after expiry
must acquire a new fenced lease without extending the original job deadline.
Broader cancellation, post-deadline reconciliation and user-facing lifecycle
commands are subsequent work.

Rollback disables new v2 plans on a migration-aware binary. Retain model/tool
history, original plan objects, Provider receipts, reservations and audit events.
Do not downgrade to an old writer, drop evidence tables, release ambiguous
ceilings, or reset a dispatched call to force another generation.
