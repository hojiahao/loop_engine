# Model capability catalogs

`providerd` can publish and activate versioned model descriptions without
restarting the service. This is optional: deployments without `catalog` retain
their static configuration. The native/compatible deployment guides remain the
authority for transport configuration, credentials, capacity and price units.

A catalog update can change the selected model, alias, capacities, capabilities,
prices and availability under an existing route. It cannot create a route,
endpoint, secret reference, principal, request policy or tool schema. Those
remain in the private deployment. No LLM-generated metadata is loaded.

## Configure and publish

Add this object to the private deployment. Use provider-owned absolute paths,
not the repository or research/holdout storage. The administrative parent must
already exist and be writable by the provider service account.

```json
{
  "catalog": {
    "directory": "/var/lib/loop-engine/provider/catalog",
    "sources": "/etc/loop-engine/provider/catalog-sources.json",
    "maximum_generations": 64,
    "trusted_keys": []
  }
}
```

Protect the source and deployment files with mode `0600`; the generation
directory must be owned by the service account with mode `0700`. Do not share
it with the invocation journal. The refresh command creates only the final
catalog directory, not an arbitrary tree of missing parents.

Create the source file below, replacing route IDs and validity timestamps with
reviewed values. `issued_at` cannot be in the future and `expires_at` must be
after publication and cover the intended run. These illustrative timestamps
are deliberately not a perpetual authorization.

```json
{
  "schema": "loop.catalog-sources/v1",
  "source_id": "local-administrator",
  "revision": 1,
  "issued_at": "2026-09-24T00:00:00Z",
  "expires_at": "2026-10-01T00:00:00Z",
  "discovery": ["research-maker"],
  "remotes": [],
  "overrides": []
}
```

`research-maker` must be a deployment model's `id`, not its friendly alias.
For a route without implemented discovery, omit it from `discovery` and use
reviewed signed metadata or an explicit local override:

```json
{"route_id": "research-maker", "availability": "active"}
```

An availability declaration records administrator authority, not a successful
live request. A route starts as `unknown` if no selected source establishes its
availability. Unknown/unavailable/retired routes cannot generate responses.

Build once, then publish and inspect with the existing deployment environment:

```bash
./scripts/pnpm.sh --filter @loop-engine/providerd build
export PROVIDERD_DEPLOYMENT=/etc/loop-engine/provider/deployment.json
node apps/providerd/dist/index.js --catalog-refresh
node apps/providerd/dist/index.js --describe
node apps/providerd/dist/index.js
```

The refresh operation only performs explicitly selected read-only discovery
and metadata requests. It does not call a generation endpoint. It prints the
immutable generation's revision and SHA-256. `--describe` prints model/policy
pins and per-model catalog status without secrets or prompts. When the service
is already running, send `SIGHUP` to its exact PID after successful publication.
`catalog_reloaded` confirms activation; `provider_catalog_reload_failed` leaves
the previous validated in-memory state intact. Inspect the administrative
inputs and preserve the failed evidence; do not delete prior generations.

For every edit, increment the source revision and set an appropriate validity
window before publishing. Reusing a revision with changed bytes, publishing an
older revision, gaps, corrupt records or a competing writer's successor fails.
After a conflict, reload the history and review the current sources before
retrying. There is no automatic retry or overwrite.

## Sources and merge precedence

1. The versioned `catalog/providers.v1.json` maps implemented protocol profiles
   to their contract suites. Deployment seeds provide explicit model IDs,
   prices and initial capabilities; this file is not a live price database.
2. Selected official model lists report availability and any native metadata.
   Discovery only narrows approved capacities/features; it never invents absent
   values or enables a feature based on a model name.
3. Each configured remote catalog must pass pinned Ed25519 verification.
4. Private administrator overrides take final precedence. An override may
   deliberately supersede discovery and is recorded as that source's authority.

Each generation retains the ordered source IDs, monotonic revisions, validity
windows and content digests. Old source watermarks remain enforced even when a
source was omitted from an intervening generation. Metadata cannot bypass the
native plugin's supported parameter combinations.

| Route | Implemented model-list operation |
| --- | --- |
| OpenAI Responses/Chat | `GET /v1/models`; IDs and announced shutdown dates, no invented capacity or price |
| Anthropic Messages | `GET /v1/models`; bounded `after_id` pagination and reported capabilities |
| Google GenerateContent/Interactions | `GET /v1beta/models`; bounded page tokens, generation-model filtering |
| Cohere V2 Chat | `GET /v1/models?endpoint=chat`; bounded page tokens and native deprecation flag |
| OpenAI-compatible and named local servers | Configured base plus `/models`, using its approved authentication |
| Anthropic-compatible | Configured base plus `/v1/models`, native Messages-list shape |
| Cloud, vendor and gateway plugins | Explicitly unavailable in this discovery implementation; configure reviewed metadata |

Unsupported discovery fails before outbound work; it does not disable the
already implemented generation transport. Listing has a maximum of eight
pages, 2,048 unique models and 512 KiB decoded bytes per response. Each request
has a five-second deadline; the refresh command has a 30-second fetch deadline.
Discovery rejects malformed records, duplicate IDs, cursor cycles, ambiguous
pagination, redirects and missing credentials. Directory scans, generation
count and JSON depth/size are also bounded.

Official protocol references: [OpenAI models](https://platform.openai.com/docs/api-reference/models),
[Anthropic models](https://platform.claude.com/docs/en/api/models/list),
[Google models](https://ai.google.dev/api/models),
[Cohere model list](https://docs.cohere.com/reference/list-models).
Metadata fields absent from the installed SDK or native response remain unknown.

## Signed remote catalogs

Add an Ed25519 SPKI public key to `catalog.trusted_keys` as
`{"id":"catalog-key","public_key":"-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----\n"}`.
Keep the corresponding private signing key outside this service and Git.
Reference that key and exact source identity in the source file:

```json
{
  "source_id": "trusted-models",
  "url": "https://catalog.example/models.json",
  "key_id": "catalog-key"
}
```

The remote response is a strict envelope:

```json
{
  "schema": "loop.signed-catalog/v1",
  "key_id": "catalog-key",
  "payload": {
    "schema": "loop.model-catalog/v1",
    "source_id": "trusted-models",
    "revision": 1,
    "issued_at": "2026-09-24T00:00:00Z",
    "expires_at": "2026-10-01T00:00:00Z",
    "entries": [{"route_id": "research-maker", "availability": "active"}]
  },
  "signature": "REPLACE_WITH_BASE64_ED25519_SIGNATURE"
}
```

Sign the exact bytes returned by `catalog_signing(payload)` from
`apps/providerd/dist/catalog-fetch.js`. This is the UTF-8 domain prefix
`loop.model-catalog-signature/v1` followed by one zero byte and canonical JSON
payload bytes. Use Ed25519 `sign(null, bytes, privateKey)` and standard base64.
Unknown keys, wrong source IDs, noncanonical signatures, tampering, expiry and
future issuance fail. Only canonical HTTPS URLs without credentials, query,
fragment or redirects are accepted. Supplier keys are never attached.

## Pins, availability and live evidence

New runs resolve the current alias to a concrete model and full capability,
price, plugin and catalog snapshot. Existing requests keep their old resolution
even after that alias points elsewhere. Restart reconstructs retained pins from
the immutable chain. Exact model IDs and fixed server-side mappings are still
required: an echoed name cannot attest unchanged model weights or cloud routing.

A changed exact model needs a complete reviewed profile: all feature flags,
context/output limits, input/output/cached prices and reasoning mode, plus the
input ceiling where the route uses it. Incomplete metadata fails instead of
inheriting another model's prices. Configure cache-creation pricing and thinking
budgets where required by the resulting model/plugin.

Availability is `unknown`, `active`, `unavailable`, `deprecated` or `retired`.
Deprecated models may still execute with explicit existing authorization;
retired models cannot be reactivated in the same catalog epoch. Retirement or
unavailability denies later dispatch of affected retained pins. Already
dispatched calls keep their captured route; reload does not cancel them.
Every pin expires with its own generation's earliest source expiry. Updating
the current catalog does not extend an old run's authority.

Verification is a separate field. `contract_verified` describes implemented
transport contracts, not a live model. To promote a model to `live_verified`,
perform an explicitly budgeted successful invocation, then add its private
journal result's 64-character filename stem as that route's `live_receipt`
override. The refresher checks the immutable response, successful finish,
usage and exact retained model/profile/deployment/plugin resolution. A list,
remote claim, receipt for a different model or changed profile cannot grant it.
This proves a recorded past invocation; it does not guarantee future
entitlement, account balance, current service health or supplier billing.
No real model has been live-verified by the offline fixture suite.

## Recovery and limits

Records are canonical, hash-linked and published by fsync plus an exclusive
filesystem link. No database migration is needed. A crash before publication
can leave a private `.pending-<uuid>` file; readers ignore it. Stop all refresh
writers before removing only such orphaned files. Never remove `.result`
records to free space or roll back a catalog.

The default limit is 64 generations, configurable up to 256. Reaching it fails
closed rather than evicting old run pins. Plan a new reviewed epoch only after
draining/finishing affected runs; preserve the old directory as audit evidence.

Metadata updates under the same deployment and executable preserve old pins.
Changing deployment authority, seed configuration or executable bytes requires
reviewed new resolutions; such changes do not silently validate old pins.
Use a new directory when transport/ACL changes make old records incompatible.
Do not advertise continuity across a binary or authority change. Keep the
prior executable, deployment and history for reproducibility.

Disable refresh writers before binary rollback. Metadata rollback within an
epoch is a new source revision and successor generation; prior bytes remain
immutable. Catalogs, invocation receipts and continuation files remain private.
The next Phase 9 platform task verifies OS-level provider/data isolation;
Phase 10 integrates these pins with run-wide lifecycle and budgets.
