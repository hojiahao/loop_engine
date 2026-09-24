# Isolated Provider runtime

The native, cloud, vendor, compatible and catalog guides describe API behavior.
This guide deploys the real TypeScript service with a restricted filesystem and
network. It does not grant data subscriptions, model entitlement or a run-wide
spending budget. Use Linux, Docker Engine, Docker Compose 2.30 or newer and the
pinned Node 24.17.0 image. The default image source is DaoCloud.

## Traffic and cost policy

Every deployment's `policy.rate` has these defaults:

```json
{
  "window_ms": 60000,
  "requests": 120,
  "tokens": 2000000,
  "maximum_usd": "20"
}
```

The sliding window bounds accepted requests, the sum of reserved input/output
tokens and reserved USD. Limits are checked after authorization/content
validation and before an invocation claim or supplier request. A denied request
does not consume a journal key; it can be retried when window capacity returns.
There is no queued wait. Reservations count admitted attempts conservatively,
including failed calls and cached replays; they are not reported as actual
supplier charges. Existing per-call token/cost/time budgets and concurrency
limits remain mandatory. The window uses monotonic elapsed time.

These limits belong to one Provider process. Restart resets them; multiple
processes have separate windows. They are not an account-wide/day/run spending
ledger. Configure one appropriately budgeted instance per allocation, and use
the subsequent Run Harness for durable aggregate reservations. Do not raise a
limit merely because the supplier returns 429; align it with actual account
limits and planned workloads. Changing policy changes the request-policy pin.

Generation retry is zero. SDK and compatible gateway controls retain that rule.
A valid completed replay reads its immutable receipt; a partial/ambiguous claim
cannot be resent automatically. Counts, generation and identity operations
remain deadline-bounded. Operators must reconcile an uncertain external charge
before authorizing another key.

`pricing.ts` calculates integer nanodollar estimates. Fresh input, cache reads,
cache writes and output use separate pinned rates. Reasoning is already included
in output and is not counted twice. Round the total up to one nanodollar.
Gateway/Guardrail/other declared fees reserve their configured ceiling.
`charged_cost` stays absent when no actual billing evidence exists. Model usage,
pricing pins and claim reserves permit downstream audit without inventing an
invoice amount.

## Prepare private deployment files

Build the locked TypeScript workspace:

```bash
./scripts/pnpm.sh install --frozen-lockfile
./scripts/pnpm.sh build
```

Set `LOOP_PROVIDER_RELEASE` to that verified build root. Compose binds only
compiled Provider/protocol files, their Node dependencies, package metadata,
the lockfile and built-in catalog. It does not mount the repository root,
Python/Rust sources, `.git`, maintenance documents, research data or host home.
Keep that build immutable while serving traffic; a changed binary requires new
resolution pins. The release directory must remain readable by UID 65532.

Create three separate private locations:

- Provider configuration, owned by UID/GID 65532 with directory mode `0700`:
  `deployment.json`, server CA/certificate/key, approved schemas, optional
  prompt artifacts and catalog source file. Individual private files use `0600`.
- Provider state, owned by UID/GID 65532 with directory mode `0700`. Journals,
  continuations and optional catalogs live here and persist across restarts.
- Egress policy, a single `0600` file owned by UID/GID 65532. It contains no
  credentials, model-routing decisions or research data.

In `deployment.json`, use container paths:

```json
{
  "port": 8091,
  "listen_address": "0.0.0.0",
  "tls": {
    "ca": "/run/provider/ca.pem",
    "certificate": "/run/provider/server.pem",
    "key": "/run/provider/server.key"
  },
  "journal": "/var/lib/provider/journal"
}
```

This is only the path fragment; the complete deployment still requires models,
principals, prices and policy. The server certificate must cover the DNS name
used by the control plane, normally `provider`. Keep the control-plane client
private key outside the Provider mount. Native host deployments continue to
bind `127.0.0.1` unless explicitly configured otherwise.

The optional catalog directory is `/var/lib/provider/catalog`; source metadata
and public signing keys belong in the private configuration. Prompts must be an
approved provider-only view, never a research or holdout artifact root. Mounting
an entire home or a parent containing data would invalidate this deployment's
isolation claim.

Put only required LLM/cloud credentials in a separate `0600` environment file
owned by the deployment operator. Compose reads it on the host using raw value
format. Never reuse an application-wide `.env` containing database, data-source
or holdout credentials. Nothing from the host environment is implicitly
forwarded except the explicit Compose entries. Do not disable TLS verification
or enable SDK debug logging. Restrict access to the Docker daemon: daemon
administrators can inspect container environments and mounts.

## Approve outbound destinations

Start with the exact origins needed by the configured plugins:

```json
{
  "schema": "loop.provider-egress/v1",
  "routes": [
    {"host": "api.openai.com", "port": 443},
    {"host": "api.anthropic.com", "port": 443}
  ]
}
```

There is no wildcard or automatic vendor-wide allowlist. Add exact Google,
cloud regional, vendor/gateway, OAuth/discovery or signed-catalog hosts only
when selected by the reviewed deployment. The egress process owns network
authority; it has no Provider credential or state mount. It accepts HTTP CONNECT
tunnels only, passes supplier TLS through unchanged and never logs payloads.
API keys remain inside TLS between the Provider SDK and supplier.

The resolver uses bounded IPv4 DNS. Loopback, link-local, private and reserved
addresses are denied by default, including unexpected private DNS results.
For an explicitly approved internal TLS server, a route may set `target_host`,
`target_port` and `allow_private: true`. Such a mapping is administrator-owned
and must never point to a data store, metadata credential endpoint or another
control service. The SDK still validates the original destination's TLS
certificate. Review and archive the policy before restarting the gateway.

The gateway bounds destinations (128), concurrent tunnels (32), connection
establishment (5 seconds), tunnel lifetime (300 seconds) and bidirectional bytes
(32 MiB). It provides no fallback, retry, TLS interception or plaintext HTTP
forwarding. This isolated profile therefore requires HTTPS supplier endpoints;
plain loopback self-hosting remains available in the separately documented
native host development profile. IPv6-only upstreams need a reviewed IPv4/TLS
front end in this deployment profile.

Use explicit credentials or approved workload-identity/token files and HTTPS
identity endpoints for cloud routes. Instance-metadata HTTP discovery is
intentionally not reachable through this profile. Mount identity files only
inside the private Provider configuration and approve the required HTTPS token
hosts. Failure to obtain identity remains an operational failure.

## Start and operate

Set these environment variables to existing absolute paths; their values are
paths, not credentials:

```bash
export LOOP_PROVIDER_RELEASE=/opt/loop-engine/releases/REVIEWED_BUILD
export LOOP_PROVIDER_CONFIG=/etc/loop-engine/provider
export LOOP_PROVIDER_STATE=/var/lib/loop-engine/provider
export LOOP_PROVIDER_ENV=/etc/loop-engine/provider-secrets.env
export LOOP_PROVIDER_EGRESS=/etc/loop-engine/provider-egress.json
docker compose -p loop-provider -f infra/compose/provider.yaml up -d --wait
docker compose -p loop-provider -f infra/compose/provider.yaml exec -T provider node dist/index.js --describe
```

Join the trusted control-plane service to the Compose `provider` internal
network (normally `loop-provider_provider`) and call `https://provider:8091`
with its pinned mTLS client identity. The profile does not publish a host port.
The internal network intentionally has no direct outside route; relying on
host port publishing on an internal bridge is not a portable deployment path.
Health remains local to each container. No application endpoint disables mTLS.

The Provider can resolve Compose peers, but external DNS forwarding is disabled.
All approved external requests use the separate egress container through Node's
environment-proxy support. The Provider has only the internal network; egress
has both internal and outbound networks. Do not attach Provider to a general
application/data network or add host networking.

The containers run as UID/GID 65532, drop Linux capabilities, enable
`no-new-privileges`, use a read-only root filesystem and have CPU/memory/PID/tmpfs
bounds. Only Provider state is writable. The Docker socket is never mounted.
These controls isolate data access; administrators still own the correctness of
the explicitly mounted prompt view, endpoint policy and secret selection.

To refresh a configured catalog, run the compiled `--catalog-refresh` command
inside the Provider container with `--use-env-proxy`, then send SIGHUP to the
Provider process using the container supervisor. Preserve the same immutable
state volume. Example for this Compose profile, whose Node process receives
signals through Docker's init:

```bash
docker compose -p loop-provider -f infra/compose/provider.yaml exec -T provider node --use-env-proxy dist/index.js --catalog-refresh
docker compose -p loop-provider -f infra/compose/provider.yaml kill -s SIGHUP provider
```

For rollback, stop admission and refresh writers, stop containers, restore the
reviewed prior release/configuration and reuse preserved state. Do not use
volume deletion or journal/catalog truncation. Reconcile ambiguous external
calls before restarting a run. Per-process rate windows reset; immutable
idempotency claims do not.

## Acceptance

```bash
./scripts/pnpm.sh --filter @loop-engine/providerd test
node --test --test-isolation=none tests/runtime/provider-boundaries.test.mjs
node --test --test-isolation=none tests/runtime/provider-egress.test.mjs
node --test --test-isolation=none tests/runtime/provider.test.mjs
```

The last command needs Docker plus root or noninteractive `sudo chown` to create
private UID-65532 fixture mounts. It restores ownership and removes only its
unique project, containers, networks and temporary directory. It uses a real
Provider, independent mTLS client, egress tunnel and synthetic HTTPS supplier;
no live API credential or paid model call is used. The tests also verify known
protected paths, absent database/holdout variables, root-write denial, blocked
direct supplier access, blocked external DNS and denied unapproved destinations.

[Docker internal networks](https://docs.docker.com/reference/compose-file/networks/#internal)
describe the deployment network boundary. The installed Node 24 CLI documents
`--use-env-proxy`; the actual supplier invocation test verifies that flag for
the native SDK path used here.
