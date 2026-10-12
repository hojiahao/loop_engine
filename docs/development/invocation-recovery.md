# Recover an existing model invocation

Use the private gRPC method
`loop.provider.v1.ProviderService/LookupInvocation` after a timeout or restart.
The same mTLS principal must own the original invocation. Follow
[Provider configuration](native-providers.md) for certificates, listener and
private journal setup. This query requires no supplier key or active model route.
It is a read-only API; an autonomous Rust recovery workflow is delivered separately.

When normal startup is blocked by an expired/missing catalog, stop the normal
Provider listener and start the compiled service with the same private deployment:

```bash
PROVIDERD_DEPLOYMENT=/absolute/private/provider.json \
  node apps/providerd/dist/index.js --recover-only
```

The command validates the existing journal and TLS/actor configuration, without
catalog activation or supplier credentials. It refuses InvokeModel/StreamModel
and never creates a missing journal. The normal startup path may create a new
journal for a new deployment; use `--recover-only` when investigating lost state.
Do not combine it with `--describe` or `--catalog-refresh`, or send SIGHUP to
reload models. After recovery, stop this listener before restoring the normal
generation service with its valid pinned catalog.

## Request

Save the exact original InvokeModel envelope and its digest **before sending it**.
Streaming requests use the equivalent InvokeModel envelope containing the same
context and invocation. Hash the UTF-8 sequence
`loop.provider-invocation/v1`, a NUL byte, and the canonical JSON of the protobuf
JSON representation (`toJson(InvokeModelRequestSchema, original)` in TypeScript).
The existing `digest_json` helper sorts object keys recursively, preserves array
order, uses compact JSON and rejects unsupported values. Do not hash protobuf
wire bytes, a new timestamp, the stream wrapper or just the prompt. Shared wire
fixtures and [the compatibility contract](../specs/protocol-compatibility.md)
define this across languages.

Send `LookupInvocationRequest` with:

- A **new** `context`: fresh request/idempotency IDs, correlation ID, current
  timestamp and the actor mapped from the client certificate. The original
  invocation's timestamp may be older than five minutes; the query's may not.
- `original_request_id` and `original_idempotency_key` from the saved command.
- `request_sha256.value`: the saved 32 digest bytes, not a hexadecimal string.
- An explicit gRPC deadline. The server caps this read at five seconds or its
  smaller configured wall-time limit.

Generated TypeScript clients expose `client.lookupInvocation(request,
{ timeoutMs: 4500 })`; Rust and Python bindings expose the same protobuf method.
This additive method's availability is the recovery capability check; there is
no feature-negotiation metadata header in this delivery.
No Authorization header, forwarded actor, holdout metadata or capability is
accepted. Lookup does not bypass the certificate principal ACL.

## Interpret the result

| Outcome | Meaning | Required caller behavior |
| --- | --- | --- |
| `ABSENT` | No claim or result observed in an existing healthy journal | Retain uncertain execution/budget state; absence alone does not authorize resend |
| `AMBIGUOUS` | Partial claim, or complete claim without a published result | Retain the reservation, surface uncertainty; do not delete the claim or create a fresh paid key |
| `COMPLETED` | Valid immutable response with matching identity | Compare the historical model resolution to the caller's pinned plan before recording success |
| `FAILED_PRECONDITION / invocation_conflict` | Original digest or completed response ID conflicts | Stop and investigate the saved identities |
| `DATA_LOSS / provider_receipt_corrupt` | Unsafe or damaged claim/result evidence | Stop and recover from a validated backup; never silently regenerate |
| `UNAVAILABLE / journal_unavailable` | Journal directory missing, inaccessible or unsafe | Restore the private journal; this is not safe absence |
| `UNIMPLEMENTED` | Server predates this additive RPC | Upgrade or stop recovery; do not fall back to InvokeModel |

Unknown/UNSPECIFIED enum values fail closed. A completed response can have a
length, refusal or tool-call finish reason; completion means a saved transport
result, not successful factor discovery or authorization to execute tools.
The response and any continuation reference remain historical: this query does
not renew an expired continuation or revive a retired model.

`reserved_cost`, when present, is an exact USD upper bound from the claim. It is
not a measured charge. It may be absent for a partial claim; keep the caller's
prior reservation. Do not release spending capacity based on missing usage or
missing `charged_cost`. Queries consume a concurrent service slot but no model
request/token/cost window; shared capacity prevents unbounded disk work.

## Reproduce and recover

The repository's `apps/providerd/test/lookup.test.ts` exercises real TLS/gRPC
against local synthetic suppliers, including restart with no supplier key.
It must observe no additional upstream call during recovery. Run the Provider
suite with `./scripts/pnpm.sh --filter @loop-engine/providerd test` on a host
permitting loopback listeners. No production credentials are required.

Back up the complete private journal with its ownership and modes. Do not copy
only result files or remove claims to force retry. A lookup never creates missing
storage or repairs evidence. To roll back, stop new recovery callers, deploy the
prior Provider binary and preserve all claims, results and caller reservations.
