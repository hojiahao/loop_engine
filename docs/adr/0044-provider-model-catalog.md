# ADR 0044: Versioned model catalogs and atomic activation

Status: implementation, 82 new catalog cases, full TypeScript regression and
`just check` and workspace build passed. Task publication and exact-commit CI
remain required. Phase 9 delivery unit 7.
The preceding compatible-route task is pushed as `9d197c7`; all seven jobs in
exact-commit CI `35958808670` passed.

## Requirement

Combine versioned built-in capability profiles, official model discovery,
trusted signed remote metadata and administrator overrides. Expose concrete
model resolutions, retain their capabilities/prices throughout a run and permit
catalog updates without restarting model conversations. Track implementation
verification separately from account availability, deprecation and retirement.

## Decision

Keep this inside the TypeScript Provider Host. The existing private deployment
defines transport addresses, secret references, principal ACLs and budgets;
catalog sources cannot replace those authority settings. No research database
access, new service or broad generic RPC is needed. `--describe` continues to
export the existing typed resolution snapshots for the control plane.

The installed, versioned plugin profiles establish implemented protocol limits,
not a fabricated list of currently available models or hard-coded live prices.
An administrator configures model routes and explicitly selects discovery
sources. Official discovery supplies bounded inventory and only capabilities
the endpoint actually reports. Missing metadata remains unknown; successful
listing does not establish a successful generation or a billing contract.
Unsupported discovery operations fail explicitly rather than guessing an URL.

Merge order is built-ins and configured seed routes, official discovery, signed
remote metadata, then local overrides. Metadata can select an exact model and
describe capacities/prices/features under an approved route; it cannot add an
endpoint, credential reference, actor, tool schema or authorization. Revalidate
the final model with the same native-plugin capability restrictions used for
normal deployment. A declared feature cannot invent an unimplemented codec.

Verify remote catalogs with pinned Ed25519 public keys over domain-separated
canonical payload bytes. Bind source ID, monotonic revision, validity window
and payload digest; reject unknown keys, changed bytes, rollback, conflicting
same-version content, expiry, redirects and unbounded responses. Remote metadata
never receives a supplier API credential. Local overrides are explicit private
administrative files, not model output.

Separate administrative publication from runtime activation. A bounded CLI
refresh validates all selected inputs and publishes an immutable, hash-linked
catalog generation in a provider-only directory. Exclusive publication of the
next generation is the compare-and-swap boundary for concurrent publishers.
Persist content and directory entries before announcing success. A partial
temporary file is not an active generation. Readers verify the complete chain;
malformed history cannot be skipped to obtain a convenient latest version.

The runtime reloads published catalogs on an explicit signal, builds all candidate
routes first and swaps one in-memory state only after validation succeeds.
Existing calls retain their selected route/plugin; subsequent calls carrying an
older retained resolution continue with its exact model and prices, never with
the current alias target. Restart loads the immutable history needed by those
pins. Retired/unavailable models and expired authority fail explicitly; there is
no automatic substitute. Bound generation count and memory use; never evict a
potentially active pin silently to admit another update.

Verification and availability are separate. An implemented or contract-tested
codec is not a live-tested model. A `live_verified` claim requires a successful
generation receipt bound to the exact configured model/profile; external lists
and signed descriptions alone cannot grant it. Preserve receipt identity and
invalidate the claim when the relevant profile changes. Infrastructure failures
remain errors, not negative model research outcomes.

## Executable acceptance

- Actual HTTP discovery fixtures for implemented native/list protocols, bounded
  pagination, native authentication, partial/malformed lists, errors and timeout.
- Deterministic precedence with provenance, unsupported capability denial,
  unknown/missing availability and no endpoint/secret override through metadata.
- Ed25519 positive case and wrong-key, tamper, rollback, conflicting revision,
  expiry, unsafe destination and duplicate-entry cases.
- Existing TLS/gRPC invocation after atomic reload and process restart, with
  unchanged old model/price pins; new aliases select only the new snapshot.
- Failed reload retains the old active state and reports an error; retirement
  denies new outbound generation, cancellation and ambiguous replay remain intact.
- Independent 2/4/8 publication processes and kill/restart on both sides of durable
  publication; no partial generation can become active or overwrite history.
- CLI describe/refresh/reload workflow, read-only catalog inspection, verification
  evidence validation and full existing Provider regression.

## Recovery and limits

Disable refresh writers before rollback. Restore the earlier executable and its
supported deployment config without deleting catalog, invocation or continuation
history. Publication rollback is a new reviewed generation, not rewriting a
past generation. Legacy static deployments retain their existing behavior when
catalog configuration is absent. Cloud/vendor discovery that has no implemented,
verified endpoint remains explicit, and live tests still require credentials and
a per-call budget. Phase 10 owns run-wide budgets and run lifecycle integration.
