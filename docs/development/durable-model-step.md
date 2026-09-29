# Run one frozen discovery model step

The authenticated Discovery API executes one administrator-approved model call,
persists its conservative token/USD reservation before dispatch, and returns a
validated canonical AST candidate. It does not evaluate or admit a factor, run
the checker model, execute tools, or implement the autonomous outer Loop.
Use development data only; protected-data authority is never granted here.
This guide describes `loop.discovery-plan/v1`. See
[controlled research context](controlled-context.md) for the separately pinned
two-call profile and its read-only tool.

## Deployment prerequisites

Stop old writers, build the workspace and apply PostgreSQL migration `0011_model_steps.sql` with
the deployment migration identity. Preserve database receipts/audit history and
the Provider journal together. Follow [runtime identity setup](runtime-authority.md)
and [Provider setup](native-providers.md) for private mTLS material.

Register the same actor at both services: `actor_id`, `authenticated_subject`,
display name and kind `AGENT`. The loopd identity has role `discovery`, an explicit
run ID, validity bounds, and the client-certificate DER SHA-256 allowlist. The
Provider principal must match that actor and the certificate used by loopd's
outbound connector. Do not forward identity in HTTP headers.

Add this optional section to the existing private `loop.runtime/v1` configuration:

```json
{
  "discovery": {
    "plan_store": "/srv/loop/private/discovery-plans",
    "plans": [{"sha256": "sha256:<plan digest>", "byte_size": 1234}],
    "provider": {
      "endpoint": "https://provider.internal:9443/",
      "domain": "provider.internal",
      "actor_id": "agent.discovery",
      "subject": "service:discovery",
      "display_name": "Discovery",
      "ca_file": "/srv/loop/private/tls/provider-ca.pem",
      "certificate_file": "/srv/loop/private/tls/discovery.pem",
      "private_key_file": "/srv/loop/private/tls/discovery-key.pem"
    }
  }
}
```

Replace the example reference with the actual bytes/digest from preparation.
The plan root must be an existing absolute canonical directory, separate from
development, protected and execution-output stores. Plan objects are flat files
named by their 64-character lowercase SHA-256, owned by the runtime UID or root,
mode `0600`/`0400`, at most 1 MiB each. No symlinks, caller-selected paths, or
automatic missing-file creation are accepted. Keep the directory private (`0700`).

## Prepare the immutable plan

Read the active Provider model snapshots and request policy without invoking a
model. This command requires a valid configured catalog, but makes no paid model
call:

```bash
PROVIDERD_DEPLOYMENT=/srv/loop/private/provider.json \
  node apps/providerd/dist/index.js --describe
```

Save that JSON privately. Select an exact `models[].snapshot` supporting strict
structured output. The `request_policy` from this output belongs in the invocation;
it is **different from** the discovery research policy derived from the plan hash.
Provider catalog/model pins must still be available when generation starts.

Use the generated Python bindings and the installed research environment to
prepare these objects (not raw Protobuf JSON bytes):

| Object | Format and required values |
| --- | --- |
| `input` | `loop.discovery.v1.DiscoveryJobInput`; development snapshot IDs and manifest digest, maker/checker snapshots, `maximum_candidates=1`; budget `maximum_steps=1`, positive token/USD limits, 120-second wall limit; omit `research_policy` |
| `invocation` | `loop.v1.ModelInvocation`; exact maker snapshot, Provider `request_policy`, system/user text only, no tools/tool choice/request ID; strict fixed schema below; for example 30-second wall limit |
| `registry` | `loop_research.operators.operator_registry().canonical_bytes` |
| `protocol` | `loop.v1.ProtocolSelectionSnapshot` described below |
| `data` | Existing development-store dataset manifest reference; do not copy datasets into the plan store |

The invocation budget must fit the job's input/output token and USD ceilings.
USD amounts use normalized decimal strings, at most nine fractional digits,
strictly positive and below one million. Invocation wall time plus five seconds
must be strictly less than the job wall time; both are at most 120 seconds. The
actual reservation also checks the remaining absolute job deadline. Submit and
execute promptly; time spent queued does not reset that deadline.

The fixed deployable schema is
[`config/schemas/discovery-ast.v1.json`](../../config/schemas/discovery-ast.v1.json).
Its formatted source is converted to compact, recursively key-sorted UTF-8 JSON
before hashing and constructing `JsonSchema`. The following code uses public
bindings and is checked against the Rust schema by a golden test:

```python
import hashlib
import json
from pathlib import Path
from google.protobuf.json_format import ParseDict
from loop.v1 import common_pb2, model_pb2
from loop_research.operators import operator_registry

document = json.loads(Path("config/schemas/discovery-ast.v1.json").read_text())
schema_bytes = json.dumps(
    document, ensure_ascii=False, sort_keys=True, separators=(",", ":")
).encode()
schema = model_pb2.JsonSchema(
    schema_id="loop.harness-factor/v1", schema_version=1,
    canonical_json=schema_bytes,
    schema_sha256=common_pb2.Sha256Digest(value=hashlib.sha256(schema_bytes).digest()),
)
provider = json.loads(Path("/srv/loop/private/provider-description.json").read_text())
# Select an approved route ID explicitly; do not silently use the first model.
selected = next(row for row in provider["models"] if row["id"] == "APPROVED_ROUTE_ID")
model = ParseDict(selected["snapshot"], model_pb2.ModelResolutionSnapshot())
request_policy = ParseDict(provider["request_policy"], common_pb2.PolicyReference())
registry_bytes = operator_registry().canonical_bytes
```

The response envelope is `{"ast": <closed AST node>}`. The service independently
parses and canonicalizes that node with the installed US-equity registry; schema
validation alone never authorizes an operator or executable code.

For `protocol`, use package `loop.v1` and exactly this sorted feature list:

```text
discovery.model-step.v1
jobs.envelope.v1
jobs.kind-input.v1
jobs.prelease-terminal.v1
provider.invocation-lookup.v1
```

Set effective limits to 4 MiB unary, 1 MiB stream, 256 KiB canonical AST,
4,096 nodes, depth 64, 500 page records, 128-byte IDs and 2,048-byte URIs.
Set current server/client build versions and SHA-256 identities from the actual
deployed builds, selection time, and SHA-256 of the exact committed
`fixtures/contracts/protocol/v1/schema.current.binpb`. Compute `selection_sha256`
using `loop_protocol.job.protocol_selection_sha256(selection)` after setting all
other fields. These are administrator pins, not evidence of online peer
negotiation. Unsupported Provider RPCs fail closed without fallback generation.

For each Protobuf object, publish `SerializeToString(deterministic=True)` bytes;
the loader requires the exact representation emitted by its generated encoder,
rejecting unknown/duplicate fields and alternate encodings. Publish JSON and
registry bytes directly. For every object compute ordinary SHA-256 of its bytes,
write once to `<plan_store>/<hex digest>`, and record
`{"sha256":"sha256:<hex>","byte_size":<exact byte count>}`. Refuse a conflicting
existing file. The plan itself is this closed JSON document:

```json
{
  "schema": "loop.discovery-plan/v1",
  "id": "discovery.baseline",
  "revision": "1",
  "actor_id": "agent.discovery",
  "run_id": "run.discovery.1",
  "provider_sha256": "sha256:<connector digest>",
  "input": {"sha256": "sha256:<input digest>", "byte_size": 123},
  "invocation": {"sha256": "sha256:<invocation digest>", "byte_size": 123},
  "registry": {"sha256": "sha256:<registry digest>", "byte_size": 123},
  "data": {"sha256": "sha256:<development manifest digest>", "byte_size": 123},
  "protocol": {"sha256": "sha256:<selection digest>", "byte_size": 123}
}
```

Compute `provider_sha256` from compact, key-sorted JSON with these exact fields:
`schema="loop.provider-connector/v1"`, `endpoint`, `domain`, `actor` (base64 of
the complete `Actor` protobuf bytes), `certificate_sha256` and `ca_sha256`
(lowercase raw SHA-256 hex of the configured PEM file bytes). Prefix the SHA-256
of that JSON with `sha256:`. Do not include private-key bytes. This binds the
same actor and TLS endpoint used at execution; replacing certificate/CA bytes
requires a new plan and preserves the old run's evidence.

Publish the plan last, then add its object reference to `discovery.plans`.
The loader fills `input.research_policy` with plan ID, revision and raw plan
SHA-256. To prepare a `StartDiscoveryRequest`, load the input protobuf and fill
that same reference. Do not rewrite the stored policy-free input template, which
would create a circular hash. Checksum/permission/schema failure denies startup.

## Use the actual gRPC API

Start the already-built server with its private database and runtime config:

```bash
loopd --database-url-file /srv/loop/private/database-url \
  --runtime-config /srv/loop/private/runtime.json
```

There is no `loopctl research run` command for this unit. Use generated
`loop.discovery.v1.DiscoveryService` clients over mTLS with explicit deadlines:

1. `StartDiscovery`: fresh `CommandContext` plus the filled, exact `discovery`
   input. The authenticated actor and run must match a configured plan. Persist
   the returned job ID/revision. Retry a lost submission with the same command
   idempotency key; do not create another job as a retry.
2. `ExecuteDiscovery`: fresh context, that `job_id`, and `expected_revision`.
   No caller model, prompt, schema, budget, data URI or lease is accepted here.
   Allow a deadline longer than the invocation's bounded execution. All three
   Discovery methods require a positive gRPC deadline of at most 120 seconds;
   missing, malformed and oversized deadlines are rejected. Concurrent stale
   revisions lose the CAS, including changes during data verification.
3. `GetDiscovery`: fresh context and job ID. This reads the latest narrow
   projection and never invokes a model. Model input/output bodies and generic
   job/holdout types are not exposed.

In Python use `DiscoveryServiceStub(channel).StartDiscovery(request, timeout=10)`,
`.ExecuteDiscovery(request, timeout=45)` and `.GetDiscovery(request, timeout=10)`
from `loop.discovery.v1.service_pb2_grpc`. `channel` must be a secure gRPC channel
with the registered client certificate/private key and trusted server CA.
All contexts use the full registered actor, fresh UUID-like request/idempotency
IDs, correlation and causation IDs, and current timestamp. All contexts expire after
30 seconds. No actor substitution or holdout capability metadata is accepted.

## Recovery and rollback

`RESERVED` means no dispatch intent committed; a valid takeover may dispatch the
saved request once. `DISPATCHED` or `AMBIGUOUS` recovery is lookup-only, using the
original immutable request ID, idempotency key and exact Provider digest. A
missing receipt is not permission to retry. A currently live lease cannot be
borrowed by another request; obtain fresh status/revision and wait for its
bounded expiry before requesting takeover. The absolute job deadline remains.

`COMPLETED` means the Provider response was saved, not that factor research
succeeded. Only a succeeded job returns a validated canonical candidate; malformed
model output is an infrastructure failure. No response releases the reserved
ceiling merely because actual charges are absent. Inspect explicit status, not
just transport completion. Use [read-only Provider recovery](invocation-recovery.md)
when the normal Provider catalog is unavailable.

To stop new work, remove/disable the discovery configuration and restart loopd.
Retain the plan objects, Provider journal, `model_steps`, command receipts and
audit events. Do not delete claims, reset reservations, or manually change
dispatch state to force a retry. The supported rollback disables this feature on
a migration-aware binary with the additive migration intact; an older binary
may reject the newer schema and is not promised write compatibility. The
old-writer guard denies generic job mutations for owned model steps. Schema
rollback is a reviewed recovery operation, not a destructive
down-migration. No paid live call or production research conclusion is implied
by the synthetic acceptance tests.
