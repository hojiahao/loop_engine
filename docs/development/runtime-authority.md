# Runtime Identity And Data Access

Phase 4 adds an optional authentication and data-publication endpoint, not the
complete research scheduler. Enable it only with explicit private configuration:
`loopd --runtime-config /absolute/runtime.json`. The HTTP health/readiness
listener never exposes job commands. No production listener is enabled by tests.

## Trust Boundary

The deployment registry maps client-certificate DER SHA-256 digests to stable
subjects, actors, one role, application validity and exact run IDs. Mutual TLS
verifies the client chain and possession of its private key. Request-body actors
must match the verified identity. Forwarded headers and bearer authorization are
not authentication mechanisms. TLS must terminate in `loopd`, not an
identity-injecting proxy. Root administrators and the host container launcher
remain trusted; this is not a boundary against a malicious host administrator.

Identity/job mappings are immutable while the listener runs. Revoke/rotate by
stopping the listener, replacing its private registry and restarting. Application
expiry is checked even on existing connections. Certificate aliases cannot turn
one subject into independent approvers. Unauthorized identities cannot probe
whether a generic job exists.

| Role | Job Access | Protected Data |
| --- | --- | --- |
| Operator | Read/cancel pinned development jobs | Denied |
| Research | Read/lease/heartbeat/complete/development views | Denied |
| Holdout worker | Corresponding protected job operations | Lease and capability required |
| Discovery, provider | No generic job operations | Denied |
| Scheduler | Internal recovery only, no new recovery RPC | Denied |

The listener registers `loop.jobs.v1.JobService`, including artifact preparation.
It does not register generic submission, holdout approval/grant RPCs or arbitrary
code execution. Job-envelope pins are administrative allowlists, not evidence of
factor correctness or data quality. Numerical execution remains unit 5 work.

## Configuration

Strict `loop.runtime/v1` JSON requires these fields:

- `bind`: explicit socket address with nonzero port.
- `server_certificate_file`, `server_key_file`, `client_ca_file`: absolute PEM
  paths. Private keys cannot be group/world accessible.
- `identities`: `actor_id`, `subject`, `display_name`, `role`,
  `certificate_sha256`, `not_before_ms`, `expires_at_ms`, `run_ids`.
- `jobs`: `{job_id, specification_sha256}` entries. This checksum covers the
  parsed, persisted `JobSpecification` Protobuf encoding, not a `JobRecord`.
- `development_store`, `protected_store`, `view_store`: distinct, non-nested,
  canonical directories. Protected/view parents must be runtime-owned 0700;
  no source may be group/world writable.
- `data`: `{job_id, manifest: {sha256, byte_size}, protected}` entries bound to
  exact immutable inputs. The request cannot select another source namespace.

Digest strings use `sha256:` and 64 lowercase hexadecimal digits. Configuration
and certificates must be runtime- or root-owned, without group/world write.
Symlinks, nonregular files, oversized/malformed JSON, duplicate identities and
invalid bounds deny startup. Provision CA material separately; private keys,
configuration and credentials must not enter Git or prompts.

The initial listener accepts its compiled descriptor and these implemented
features: `jobs.envelope.v1`, `jobs.kind-input.v1`, `jobs.prelease-terminal.v1`.
Older stored selections remain immutable; they are not silently reinterpreted as
current executable jobs. Accepting an older descriptor requires an explicit
compatibility change/test. The additive data RPC does not change factor IDs.

## Lease-Bound Publication

Protected acquisition verifies the persisted consumed grant, terminal period,
approval attachments, frozen batch and exact plan entry. A currently owned lease
receives an opaque token in binary `loop-holdout-capability-bin` metadata, bound
to server instance, subject, job, lease and expiry, for at most five minutes.
Tokens never enter jobs, receipts, audit events or command-line arguments.

Protected reads and worker commands require this token. An old acquisition
receipt cannot issue authority for an expired or replacement lease. Restart
invalidates tokens. A still-current lease owner can retry acquisition to receive
a new token without consuming a grant again. Consumed grants never reopen.

`PrepareJobArtifacts` requires actor, job/lease IDs, expected revision, idempotency
key and fresh request time. The broker checks actual canonical declarations,
schemas and bytes, copies only authorized payloads into private staging, then
commits acceptance/receipt/audit in one PostgreSQL transaction. Fresh lease/file
checks precede no-replace publication of the read-only leaf directory.

Limits: 128 unique artifacts, 256 MiB per view, two simultaneous publications per
broker, 30-second preparation deadline. Replay rechecks identity, bytes and lease.
Failures remain operational errors, not factor rejections. Replies contain
artifact references and an opaque `view_id`, never large payloads or secrets.
Existing corrupted views cannot be overwritten by retry.

Acceptance is not proof of delivery. A killed publisher can leave private staging;
retry revalidates and publishes the same complete view without duplicate audit.
Staging is never a worker mount. Clean it only after stopping the publisher and
reviewing exact project-owned paths. Do not scan/delete arbitrary temporary data.

## Process Isolation

`infra/compose/runtime-workers.yaml` supplies deny-by-default worker profiles:
non-root, read-only root, no capabilities, no privilege escalation, bounded
memory/PIDs/scratch and no network. Discovery/provider mount no research storage.
Numerical profiles mount only one runtime-selected leaf at `/data`, read-only.
They receive no parent/source mount, database secret, registry or engine socket.

Only trusted orchestration may set `LOOP_WORKER_VIEW`, after fresh runtime lease
and capability validation. A supplied path or cached receipt is not permission.
Automatic scheduling and egress-enabled LLM containers belong to the later
Harness/Loop Runtime phases; this profile does not implement them. Do not expose
a generic container launcher to Agent tools. Revocation cannot make already
disclosed bytes unknown: stop affected workers and discard private scratch.

Development files are restricted to IS/development sample roles. Protected
declarations accept only explicit synthetic quality in this phase. Production
PIT validation, licensed data and real holdout computation remain later gates.

## Verification And Rollback

Run `just check`, `just test`, `just build`, `just doctor`, and
`just test-isolation`. The last command exercises four real containers, known
protected paths and forbidden writes, then removes its own containers/files.
The clean-container gate invokes it on the host; test workers never receive the
Docker socket. The Rust suite covers real TLS, files, PostgreSQL, independent
2/4/8-process publication and commit-boundary kill/restart.

Rollback disables the optional listener/publisher and stops its workers. Preserve
jobs, grants, receipts, audit and immutable sources. There is no new database
migration or destructive down-migration. Deployment remains opt-in.
