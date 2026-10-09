# Persistent research run CLI

`loopctl run` operates one human-owned, administrator-pinned research run. The
run shares a durable budget across its Discovery children. This workflow does
not perform numerical evaluation, semantic checking, factor admission or
unattended scheduling; those are separate integrations.

## Operator configuration

The server administrator registers two independent identities: a Human operator
who owns the run and a Discovery Agent whose frozen plan executes its children.
An actor label in JSON is attribution, not authentication: the server also checks
the presented mTLS certificate against its identity registry. The operator gets
no database password, Provider credentials or holdout capability.

Use the explicit `loop.operator/v1` profile. The existing `loop.client/v1`
Discovery profile is Agent-only and cannot be used with these commands. A Human
profile is likewise rejected by `loopctl discovery`.

```json
{
  "schema": "loop.operator/v1",
  "endpoint": "https://127.0.0.1:18443",
  "server_name": "loopd.internal",
  "ca_file": "/srv/loop/operator/ca.pem",
  "certificate_file": "/srv/loop/operator/human.pem",
  "private_key_file": "/srv/loop/operator/human.key",
  "actor": {
    "actor_id": "actor.operator.hojiahao",
    "subject": "human:hojiahao",
    "display_name": "hojiahao"
  }
}
```

These paths and identity names are examples, not claims that the operator has
been provisioned in production. Configuration and private key files must have
mode `0600` and be owned by the current user or root. All files use absolute,
canonical paths without symlinks; certificates cannot be writable by group or
others. Unknown JSON fields, insecure endpoints and TLS verification bypasses
are rejected. Transport/server error details are never printed.

## Administrator plan preparation

First prepare a frozen Discovery plan using the
[Discovery plan guide](durable-model-step.md), or its
[controlled-context variant](controlled-context.md). The new run uses that
plan's exact run ID, filled `DiscoveryJobInput`, protocol selection and outbound
Agent identity. The source Discovery input template omits `research_policy`;
the input copied into a run specification must include the reference derived
from the published Discovery plan. Do not confuse this reference with the
separate run-plan reference below. Prepare a new run before submitting any
standalone Discovery jobs with its ID.

Register the Human owner's distinct actor and client-certificate fingerprint in
the deployment identity registry with `role: "operator"` and the explicit run
ID. The Discovery identity keeps `role: "discovery"`, the same run ID and the
actual configured Provider connector actor. These identities must have different
actor IDs and authenticated subjects; an existing Agent certificate cannot
become a Human identity by editing the CLI JSON. Identity validity intervals
must cover the intended execution period.

Create a separate private flat directory, for example
`/srv/loop/private/run-plans`, with mode `0700`. Prepare a canonical binary
`loop.runs.v1.RunSpecification` using the generated bindings:

| Field | Required value |
| --- | --- |
| `plan` | Omit from the stored template to avoid circular hashing |
| `run_id` | Exact run ID in the existing Discovery plan |
| `owner` | Full registered Human actor, including kind, ID, subject and display name |
| `executor` | Full registered Agent actor used by the Discovery Provider connector |
| `discovery` | Exact filled `DiscoveryJobInput`, including its research policy |
| `protocol_selection` | Exact selection stored in the Discovery plan |
| `maximum_rounds` | Integer from 1 to 64 |
| `budget` | Explicit cumulative steps, input/output tokens, exact USD and wall-time ceilings |

Run wall time is positive, at most 30 days, and uses millisecond precision. Each
child keeps its original budget. For a two-round run, total token/step/USD
ceilings must fit two full child allowances; allow sufficient wall time for both
children plus queue and orchestration time. A budget that fits only one child
stops without creating a second one. This is a conservative reservation policy,
not a prediction of actual token usage or fees.

Serialize the template with `SerializeToString(deterministic=True)` in Python,
or the equivalent generated Protobuf encoder. The import is
`from loop.runs.v1 import service_pb2`; shared IDs, actors, money and policy
references come from `loop.v1.common_pb2`. Compute ordinary SHA-256 of the exact
template bytes. Publish them once at `<plan_store>/<64-character hex digest>`
with mode `0600` or `0400`, and record their exact byte count. Refuse an existing
file with different bytes; never overwrite an immutable plan object.

Next publish this closed JSON wrapper in the same content-addressed directory:

```json
{
  "schema": "loop.research-run/v1",
  "id": "research.baseline",
  "revision": "1",
  "specification": {
    "sha256": "sha256:<template digest>",
    "byte_size": 1234
  }
}
```

Replace the digest and byte count with the template's actual values. The ID uses
lowercase policy-ID syntax; revision is a positive canonical decimal integer.
Hash the exact UTF-8 wrapper bytes, then publish those bytes at the wrapper's
own hex-digest filename. Formatting affects this byte hash: do not reformat the
wrapper after publication. Each object is at most 1 MiB.

Add the wrapper reference to the existing private `loop.runtime/v1` JSON:

```json
{
  "runs": {
    "plan_store": "/srv/loop/private/run-plans",
    "plans": [{"sha256": "sha256:<wrapper digest>", "byte_size": 456}]
  }
}
```

This is an additional section, not a complete runtime configuration. Preserve
its existing identity, TLS, storage and `discovery` sections. The catalog accepts
1–64 plans and rejects duplicate run IDs. A run catalog requires the matching
Discovery executor. Restart the service after adding the plan; loading validates
content, identity and exact Discovery bindings without making a model call.

Finally prepare the operator's private `run-plan.binpb`: encode a
`loop.v1.PolicyReference` with the wrapper's ID, revision and raw 32-byte wrapper
SHA-256. This is the file consumed by `loopctl run start --plan`; it is **not**
the JSON wrapper or the `RunSpecification` template. The server fills that same
reference into its resolved specification. Keep all these configuration files
outside Git and outside research data/output directories.

## Start, step and observe

The administrator provides a private canonical binary `loop.v1.PolicyReference`
file identifying a registered immutable run plan. It contains `policy_id`, a
positive decimal revision and the 32-byte SHA-256 digest. Its encoding must match
the generated Protobuf encoder exactly; unknown or duplicate fields and
nonminimal encodings fail before network access. The file must be absolute,
regular, at most 4 KiB, non-symlinked and mode `0600`. It is a reference, not a
replacement plan or a way to override the frozen budget.
The registered plan also fixes the run ID. Start refuses to adopt pre-existing
unbudgeted jobs with that ID; supplying a different command key cannot replace
the plan or replenish an existing run's allowance.

```bash
loopctl run --config /srv/loop/operator/run-client.json start \
  --plan /srv/loop/operator/run-plan.binpb --key start.research.001

loopctl run --config /srv/loop/operator/run-client.json status \
  --run run.research.001

loopctl run --config /srv/loop/operator/run-client.json step \
  --run run.research.001 --revision 1 --key step.research.001
```

Use the run ID and revision returned by the preceding response. The example
revision is not a promise that later runs use the same value. Start reserves the
first entire child allowance. Each step executes or recovers the current child
and advances at most one round. A successful child may reserve and queue the
next child; a separate explicit step is needed to execute that child. There is
no automatic loop or retry in the CLI.

For a lost response, retry the exact original command, key and revision.
Receipts return the original command's historical result, even if the run has
since advanced; use `status` for its current state. They prevent that retry from
authorizing another round. Never generate a new
key or update the revision automatically to circumvent a conflict. Status reads
are observational and do not reset budgets. Plan changes or inaccessible
execution evidence fail closed; authorized stop-only status can return
`plan_verified: false` without exposing candidates or granting execution.

## Output and budgets

Successful transport responses print one `loop.run-cli/v1` JSON document to
stdout. The `run` object includes status, revision, frozen maximum rounds,
completed rounds, current child handle, original budget, cumulative reservations,
submission/update/deadline timestamps and `plan_verified`.

All integer fields are decimal strings, including revisions, token counts,
round counts and timestamp components. USD values remain exact canonical decimal
strings. The CLI rejects unknown status values, inconsistent projections and
reservations exceeding their ceilings instead of printing a misleading result.

Reservations are conservative ceilings, not billing totals. The complete frozen
child budget is reserved once at the run level; individual model steps consume
that child's budget. Unused or ambiguous calls do not refund parent reservations.
Both restart and repeated commands preserve the original absolute deadline.

| Exit code | Meaning |
| --- | --- |
| 0 | Valid active or completed run response |
| 2 | Invalid arguments, configuration or plan input |
| 3 | Authentication or authorization denied |
| 4 | Revision, idempotency or state conflict |
| 5 | Client/RPC deadline reached |
| 6 | Transport/server evidence unavailable, rate limited or remotely cancelled |
| 7 | Run exhausted its budget, failed infrastructure, or exceeded its deadline |
| 8 | Missing, corrupt or unsupported response, or internal client failure |
| 130 | Client interrupted by SIGINT |

Errors print a redacted `loop.run-cli/v1` envelope to stderr. A terminal run
response still prints its safe projection to stdout even when it exits with 7.
SIGINT only drops the client RPC; it does not claim to cancel the durable run or
refund reservations. `--timeout-seconds` bounds connection plus RPC to 1–120
seconds (default 30); a deadline does not authorize resending a model invocation.

## Disable and recover

Remove the optional `runs` section from the deployment configuration and restart
the service to stop new starts and advancement while retaining owner-scoped
status. An empty `runs.plans` list is invalid. Preserve the database, immutable plans,
child journals, command receipts and audit evidence. Existing Discovery control
commands require the registered Agent identity; changing an operator JSON actor
does not grant that role. Run-level pause/cancel/resume and background scheduling
are subsequent delivery units.

Migration 0015 is additive. Disable writers before a deployment rollback, keep a
migration-aware binary and preserve research history. Do not deploy an older
writer over the new schema or perform destructive down-migrations. The design
and verification requirements are recorded in
[ADR 0051](../adr/0051-persistent-research-runs.md).
