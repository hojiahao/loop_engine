# Operate a bounded Discovery job

`loopctl discovery` operates administrator-approved Discovery plans through
mutual TLS. It has no database connection, model API key or holdout authority.
The output is a candidate for later deterministic research, never factor
admission or a claim of investment performance.

For the deployed versioned-directory layout, systemd unit, production database
upgrade and recovery procedure, see [production cutover](production-cutover.md).

## Install and configure

Build the locked workspace on a bootstrapped host:

```bash
./scripts/cargo.sh build --locked --offline -p loopd -p loopctl --bins
install -d -m 0755 /srv/loop/bin
install -m 0755 .tools/target/debug/loopd .tools/target/debug/loopctl /srv/loop/bin/
```

Choose the actual deployment prefix. The example installs development-profile
binaries; production releases should install the verified release artifacts.
No compiler or project source tree is required by the installed client binary.

Provision the private server configuration and frozen plan using
[runtime identity](runtime-authority.md), [Provider setup](native-providers.md),
[plan preparation](durable-model-step.md) and [the two-call profile](controlled-context.md).
Use the same registered Discovery actor at both services. Publish the plan before
starting work; the client cannot replace its model, prompt, schema or budget.

Keep the deployment schema-owner database file separate from the runtime file.
Stop old writers, take a recoverable database backup and apply all pending
migrations with the deployment identity:

```bash
/srv/loop/bin/loopd --database-url-file /srv/loop/private/migration-database-url \
  --database-schema public --migrate
/srv/loop/bin/loopd --database-url-file /srv/loop/private/runtime-database-url \
  --database-schema public --check-database
/srv/loop/bin/loopd --database-url-file /srv/loop/private/runtime-database-url \
  --database-schema public --bind 127.0.0.1:8080 \
  --runtime-config /srv/loop/private/runtime.json
```

The schema argument selects a trusted deployment namespace, not a per-command
research input. Migration and runtime must name the same namespace. TLS remains
required in the private PostgreSQL URL. The client never reads either file.
`loopd` accepts SIGINT and SIGTERM for graceful service shutdown; use a bounded
service-manager stop timeout. Forced termination preserves committed receipts.

Write `/srv/loop/private/client.json` with mode `0600`:

```json
{
  "schema": "loop.client/v1",
  "endpoint": "https://loopd.internal:8443/",
  "server_name": "loopd.internal",
  "ca_file": "/srv/loop/private/tls/runtime-ca.pem",
  "certificate_file": "/srv/loop/private/tls/discovery.pem",
  "private_key_file": "/srv/loop/private/tls/discovery-key.pem",
  "actor": {
    "actor_id": "agent.discovery",
    "subject": "service:discovery",
    "display_name": "Discovery"
  }
}
```

Paths must be absolute canonical regular files owned by the current user or root.
Configuration and private keys cannot be accessible to group/others. Do not put
API keys into this configuration. Certificates and the server identity registry
provide authority; changing the actor fields does not impersonate another user.
Symlinks are rejected in every path component. CA/certificate/input files must
not be writable by group/others. Configuration is limited to 64 KiB, each TLS
file to 128 KiB and the input to 1 MiB. HTTPS certificate verification and client
authentication are mandatory; the endpoint cannot include user information,
a query, a fragment or a path other than `/`.

Prepare `discovery-input.binpb` from the frozen input template as described in
the plan guide. Fill its research policy with the published plan ID, revision
and SHA-256 without changing the stored template. Serialize deterministic
Protobuf bytes and save the bounded input privately. JSON is not accepted as
Protobuf input. The server independently matches the complete input to its plan.

## Commands and outputs

```bash
loopctl discovery --config /srv/loop/private/client.json start \
  --input /srv/loop/private/discovery-input.binpb --key submit.experiment.1
loopctl discovery --config /srv/loop/private/client.json status --job JOB_ID
loopctl discovery --config /srv/loop/private/client.json execute \
  --job JOB_ID --revision 1 --key execute.experiment.1
loopctl discovery --config /srv/loop/private/client.json events \
  --job JOB_ID --after 0 --limit 100
```

Place the required `--config` before the subcommand, as in the examples.
Replace `JOB_ID` and the revision with observed values. A returned JSON envelope
uses `schema: "loop.discovery-cli/v1"` and includes the current job handle and,
where applicable, step evidence. Job/step states and event operations use
lowercase names such as `paused`, `infrastructure_failed` and `tool_record`.
Integer identities and counters are decimal strings to avoid JavaScript rounding;
timestamps carry decimal-string `seconds` and `nanos`. `reserved_cost` is either
`null` when no money reservation exists or an object with the exact decimal
`amount` string and `currency_code: "USD"`. Reservations are ceilings, not invoices.
`plan_verified=false` is stop-only observation: the stored metadata is readable,
but no candidate or execution authority is returned.
A returned candidate carries `expression_id`, `canonicalization_profile` and
`canonical_json`. The last field is a UTF-8 string preserving the original
canonical JSON bytes, rather than a reserialized AST object, so its identity can
be independently verified. Parse that string separately when an AST is needed.

Every mutation requires its original explicit key. Retrying a lost response uses
the same key, job, revision and input; the CLI refreshes transport request ID/time
while preserving semantic correlation. Never change the key to force a retry.
The CLI does not silently refresh stale revisions or retry model generation.
Execute/takeover obeys the existing lease boundary: a live handler's execution
right cannot be borrowed, and a dispatched request is never sent again.
Execute does not store the caller key as a separate command receipt. If execution
advanced the job revision before its response was lost, repeating the old
revision returns a conflict. Read status explicitly before choosing a recovery
operation; the key never bypasses revision or lease checks.

Status and events are read-only. Event pages contain only sequence, time and
typed lifecycle operations for that authorized job. Continue using
`next_after_sequence` while `has_more` is true; the cursor cannot be borrowed from
another job. This filtered projection is not a full global audit-chain proof.
The default cursor is zero and the page size defaults to 100; `--limit` accepts
1 through 100. Events output `job_id`, `events`, `next_after_sequence` and
`has_more` instead of a job handle.

Use `--timeout-seconds` before the subcommand, from 1 through 120 (default 30).
SIGINT stops the client wait with exit 130. It does **not** claim the durable job
was cancelled: observe status and issue an explicit cancellation if required.

| Exit | Meaning |
| --- | --- |
| 0 | Command accepted or readable nonfailed state |
| 2 | Invalid arguments, input or local configuration |
| 3 | Authentication or authorization denied |
| 4 | Revision/idempotency conflict or failed precondition |
| 5 | Deadline exceeded |
| 6 | Transport unavailable, rate limit or remote RPC cancellation |
| 7 | Returned job is rejected, failed, cancelled or budget-exhausted |
| 8 | Job not found, invalid response or internal/protocol error |
| 130 | Client interrupted; durable outcome is not inferred |

A successful `cancel` returning Cancelled therefore exits 7, with its successful
response on stdout. RPC completion does not mean research success. Static error
categories go to stderr; raw server/transport errors, secrets and prompts do not.
Errors have the shape `{"schema":"loop.discovery-cli/v1","error":{"category":"timeout"}}`
and leave stdout empty. The stable categories are `arguments`, `configuration`,
`input`, `authentication`, `authorization`, `conflict`, `timeout`, `transport`,
`rate_limit`, `remote_cancelled`, `not_found`, `protocol`, `internal` and
`interrupted`.

## Pause, restart and recover

```bash
loopctl discovery --config /srv/loop/private/client.json pause \
  --job JOB_ID --revision CURRENT_REVISION --key pause.experiment.1
loopctl discovery --config /srv/loop/private/client.json resume \
  --job JOB_ID --revision PAUSED_REVISION --key resume.experiment.1
loopctl discovery --config /srv/loop/private/client.json cancel \
  --job JOB_ID --revision CURRENT_REVISION --key cancel.experiment.1
loopctl discovery --config /srv/loop/private/client.json reconcile \
  --job JOB_ID --revision CURRENT_REVISION --key reconcile.experiment.1
loopctl discovery --config /srv/loop/private/client.json expire \
  --job JOB_ID --revision CURRENT_REVISION --key expire.experiment.1
```

Pause removes the lease; Resume acquires a fresh fenced lease against the same
absolute deadline, original request, counters and reserved budget. An expired,
cancelled or failed job cannot resume. After process restart, inspect status:
use Resume for Paused, Execute for Queued or an expired lease still within the
job deadline, and Reconcile for late evidence on supported paused/terminal jobs.
Do not create a replacement job as crash recovery.

Reconcile performs one bounded receipt lookup, never a tool or paid invocation.
It preserves cancellation/failure, reservations and candidate absence. Missing
Provider evidence is not permission to invoke again. After the absolute deadline,
Expire can record budget exhaustion; no unattended expiry sweeper is claimed.

## Stop-only deployment and rollback

Stop `loopd`, remove its optional `discovery` section and restart the same
migration-aware binary while retaining the owner identity and storage config.
Status, scoped events and stop commands remain available even if plan files are
unavailable. Execute/Resume/Reconcile require their original supported plan and
connector; stop-only mode cannot bypass those requirements.

Preserve the PostgreSQL state, immutable plan objects, Provider journal, audit
and every ambiguous reservation. Disable new writers before rolling back.
Retain additive schema/index changes; never drop evidence, reset retries or
rewrite a dispatched request to Reserved. No paid downloads or live model calls
are required for the synthetic CLI acceptance suite.
