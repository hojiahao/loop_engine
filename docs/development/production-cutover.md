# First production deployment and subsequent cutovers

The owner authorized deployment to `117.50.81.155`. The pre-deployment inventory
reported an existing `loop_engine` PostgreSQL database at migrations 1 through 5
and no running Loop application service. This is the first application install,
not a migration of a running research workload. Recheck these facts immediately
before applying changes; this document is a procedure, not a deployment receipt.
The executed first-install receipt is in
[Phase 10 operational verification](../verification/phase-10-discovery-operations.md).

Install `loopd` and `loopctl` with Discovery execution disabled. No Provider
deployment, model credential, administrator-approved execution plan or licensed
research dataset is implied. A healthy process and database do not establish a
running autonomous research loop. Client commands and certificate/configuration
formats are documented in [the Discovery CLI guide](discovery-cli.md).

## Inventory and backup

Use the approved SSH host and local administrator/peer authentication. Preserve
the shared PostgreSQL installation, unrelated databases and existing listeners.
Check `ss -ltn` before selecting ports: prefer loopback HTTP `8080` and mTLS
`8443` when free, otherwise use loopback `18080` and `18443` consistently in the
unit and private configuration. Do not open a public database or RPC listener.

Run the following using `sudo -u postgres psql -X --no-password -d loop_engine`.
These checks read only metadata and counts; no request, audit payload or secret
is printed. The exact migration checksums must also match the checkout used to
build the deployment bundle.

```sql
BEGIN READ ONLY;
SET LOCAL statement_timeout = '30s';
SELECT current_database(), current_setting('server_version');
SELECT version, success, encode(checksum, 'hex') AS checksum
FROM public._sqlx_migrations ORDER BY version LIMIT 100;
SELECT count(*) FILTER (WHERE kind IN (3, 7) AND state = 4) AS blocks_0006,
       count(*) FILTER (WHERE kind = 3 AND state = 5) AS blocks_0007,
       count(*) FILTER (WHERE kind IN (2, 3)) AS blocks_0009
FROM public.jobs;
SELECT 'jobs' AS relation, count(*) FROM public.jobs
UNION ALL SELECT 'command_receipts', count(*) FROM public.command_receipts
UNION ALL SELECT 'audit_events', count(*) FROM public.audit_events;
SELECT sequence, encode(event_sha256, 'hex') AS event_sha256
FROM public.audit_events ORDER BY sequence DESC LIMIT 1;
SELECT audit_ledger_id, last_observed_at_ms FROM public.store_metadata;
SELECT conname, pg_get_constraintdef(oid)
FROM pg_constraint WHERE conrelid = 'public.jobs'::regclass
  AND conname IN ('jobs_state_check', 'jobs_check3');
SELECT usename, application_name, state, count(*)
FROM pg_stat_activity
WHERE datname = current_database() AND pid <> pg_backend_pid()
GROUP BY usename, application_name, state;
COMMIT;
```

For this upgrade, all three `blocks_*` counts must be zero. Migrations 6 and 7
refuse historical backtest success/rejection without its required evidence;
migration 9 refuses pre-existing factor-evaluation/backtest jobs without verified
trial history. A nonzero count requires a separately reviewed evidence import.
Deleting records or disabling these guards is not a recovery procedure.

Confirm application writers are absent or stopped before backup and migration.
Keep the original job/receipt counts and final audit sequence/hash as comparison
evidence. This summary does not prove the entire audit chain; it detects changes
to the expected unchanged cutover snapshot.

Create a new private backup directory for this deployment. The example ID is a
placeholder; choose a fresh UTC timestamp and do not overwrite an older backup.
The archive is written locally on the database host without exposing a password.

```bash
loop_cutover_id=YYYYmmddTHHMMSSZ
sudo install -d -o postgres -g postgres -m 0700 /var/lib/postgresql/loop-backups
sudo install -d -o postgres -g postgres -m 0700 "/var/lib/postgresql/loop-backups/$loop_cutover_id"
sudo -u postgres pg_dump --no-password --format=custom --dbname=loop_engine \
  --file="/var/lib/postgresql/loop-backups/$loop_cutover_id/loop_engine.dump"
sudo -u postgres chmod 0600 "/var/lib/postgresql/loop-backups/$loop_cutover_id/loop_engine.dump"
sudo -u postgres pg_restore --list "/var/lib/postgresql/loop-backups/$loop_cutover_id/loop_engine.dump"
sudo -u postgres sha256sum "/var/lib/postgresql/loop-backups/$loop_cutover_id/loop_engine.dump"
```

Check every exit code before migration. Archive listing verifies readability,
not full restoration. A restore drill uses a separate disposable database and
never overwrites production. Preserve any existing immutable artifact stores,
private deployment configuration and Provider journals separately; `pg_dump`
contains only the database. New deployments must record that these stores were
absent rather than inventing a previous application release.

## Forward migration and grants

Generate `node scripts/postgres-production-bundle.mjs` from the exact release
source and save its checksum with the deployment receipt. Apply its output with
administrator `psql -X --no-password -v ON_ERROR_STOP=1 -d loop_engine` on the
approved host. The bundle contains its own `BEGIN`/`COMMIT`; do not split it into
individual migration invocations or add an independent transaction wrapper.

The bundle requires database `loop_engine`, switches to `loop_engine_owner`,
uses schema `public`, and shares the application's schema advisory lock. It
checks already-applied SQLx SHA-384 checksums, applies pending migrations,
records migration receipts and updates application grants in one transaction.
Statement/lock waits are bounded at 30/5 seconds. Any SQL error stops `psql`; its
connection closing rolls back the uncommitted upgrade. Investigate the error
before retrying the same bundle. Neither the bundle nor an ordinary runtime
startup upgrades PostgreSQL itself.

| Migration | Upgrade behavior and compatibility |
| --- | --- |
| 6–7 | Add immutable backtest evidence and atomic result/rejection constraints; historical blockers above must be absent. |
| 8–9 | Add revision-fenced perturbation/factor state and immutable trial history. Old writers cannot omit mandatory trial receipts. |
| 10 | Add numerical evidence; historical successes retain an explicitly unverified baseline rather than fabricated metrics. |
| 11–12 | Add model/tool evidence and a two-ordinal key. Model writer tokens advance from `v1` to `v2`. |
| 13 | Add retry/response chronology and Discovery Paused state; preserve original bytes and reservations. Require writer token `v3` for model/tool changes and all updates to Discovery jobs, including jobs with no model step yet. |
| 14 | Add only the partial `audit_events(job_id, sequence)` index. |

Migration 13 temporarily disables the model protection trigger only inside the
migration transaction to populate response chronology, then re-enables it.
Stop old writers beforehand; setting a writer token manually is not permission
to reuse an old implementation. Runtime verifies the complete migration count
and exact checksums, so a previous binary can refuse the upgraded database even
when only additive changes were applied.

The bundle grants SELECT/INSERT on application tables and UPDATE only on
`store_metadata`, `jobs`, `holdout_periods`, `holdout_grants`,
`perturbation_states`, `factor_states` and `model_steps`. It revokes migration
INSERT/UPDATE/DELETE. New tables need no sequence privileges. Existing default
privileges do not grant the UPDATE rights, so applying raw SQL files alone is
insufficient. The bundle does not repair unrelated over-broad historical grants;
check the resulting effective permissions explicitly:

```sql
BEGIN READ ONLY;
SET LOCAL statement_timeout = '30s';
SELECT rolname, rolsuper, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls
FROM pg_roles WHERE rolname IN ('loop_engine_owner', 'loop_engine_app');
SELECT pg_has_role('loop_engine_app', 'loop_engine_owner', 'MEMBER') AS owner_member,
       has_schema_privilege('loop_engine_app', 'public', 'CREATE') AS schema_create;
SELECT c.relname,
       has_table_privilege('loop_engine_app', c.oid, 'SELECT') AS can_read,
       has_table_privilege('loop_engine_app', c.oid, 'INSERT') AS can_insert,
       has_table_privilege('loop_engine_app', c.oid, 'UPDATE') AS can_update,
       has_table_privilege('loop_engine_app', c.oid, 'DELETE') AS can_delete,
       has_table_privilege('loop_engine_app', c.oid, 'TRUNCATE') AS can_truncate
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p') ORDER BY c.relname;
SELECT version, success, encode(checksum, 'hex') AS checksum
FROM public._sqlx_migrations ORDER BY version LIMIT 100;
SELECT tgname, tgenabled FROM pg_trigger
WHERE tgrelid IN ('public.jobs'::regclass, 'public.model_steps'::regclass,
                 'public.tool_results'::regclass, 'public.audit_events'::regclass)
  AND NOT tgisinternal ORDER BY tgname;
SELECT indexdef FROM pg_indexes
WHERE schemaname = 'public' AND indexname = 'audit_events_job_sequence';
COMMIT;
```

Require successful versions 1–14, no owner membership/schema CREATE, no runtime
DELETE/TRUNCATE, migration metadata INSERT/UPDATE denied, and UPDATE only on
the mutable allowlist. All ordinary triggers must be enabled (`O`). Compare the
pre-upgrade job/receipt counts and final audit head again before starting writers.

## Install and activate

Stage both binaries under `/srv/loop/releases/RELEASE_ID/bin`, owned by root
and executable by the dedicated service account. Record SHA-256 digests,
architecture, dynamic-library requirements, source identity and the descriptor
digest. Use the complete Git commit for a clean release. A build from an
uncommitted worktree needs a distinct ID and source-change/artifact digests;
labelling it only with its base commit would misidentify its contents.

Keep `/srv/loop/private/runtime-database-url` outside the release directory.
The file must be mode `0600`, readable by the service account, and name
`loop_engine_app` with required TLS. Never pass its content as a command-line
argument. Run the staged binary's `--check-database` as the service account with
that file and `--database-schema public` before activation. This checks actual
runtime TLS, session settings, ledger identity and compiled migration checksums.

The initial default-deny service may omit `--runtime-config`. For stop-only mTLS,
add `/srv/loop/private/runtime.json` with the chosen loopback mTLS port, private
server key, trusted client CA and a nonempty registry containing the specifically
registered Discovery actor and run. Omit `discovery` and every research executor
section. `jobs` and `data` can be empty. Use three distinct service-owned private
directories for development, protected and view stores. Keep the CLI client
configuration and client private key root-owned and mode `0600`; the daemon
does not need those files. Certificate identity and actor/run registration must
match; a CA-signed but unregistered client is denied.

Use `loopd.service` with a dedicated non-login account, an explicit working
directory and an executable path under `/srv/loop/current/bin/loopd`:

The versioned [service unit](../../infra/systemd/loopd.service) selects
`18080`/private runtime configuration because this host's 8080 was already used.
It also restricts filesystem writes to the data directory and hides operator
credentials from the daemon. The following is the minimal configurable form:

```ini
[Unit]
Description=Loop Engine persistent orchestration
After=network.target

[Service]
Type=simple
User=loopd
Group=loopd
WorkingDirectory=/srv/loop
ExecStart=/srv/loop/current/bin/loopd --database-url-file /srv/loop/private/runtime-database-url --database-schema public --bind 127.0.0.1:8080 --runtime-config /srv/loop/private/runtime.json
Restart=on-failure
RestartSec=3
TimeoutStopSec=30
KillSignal=SIGTERM
UMask=0077
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

Replace the account/port with the provisioned values before installation. Omit
the runtime-config argument only for the initial HTTP readiness-only stage.
Grant that account filesystem traversal and access to daemon files without
making private keys group/world readable. Do not place canonical private
configuration paths underneath the `current` symlink; loaders reject symlinks.

After checking the staged release, record the previous `current` target if one
exists. Create a new uniquely named symlink in `/srv/loop` to the complete release
directory, then atomically rename that symlink to `/srv/loop/current` on the same
filesystem. Never copy over files used by a running process. For the first
installation there is no previous release to restore. Reload the systemd unit,
enable/start the service, then record:

```bash
systemctl is-active loopd.service
systemctl show loopd.service -p MainPID -p ExecMainStatus -p NRestarts
curl --fail --silent --show-error --max-time 5 http://127.0.0.1:8080/healthz
curl --fail --silent --show-error --max-time 5 --output /dev/null \
  --write-out '%{http_code}\n' http://127.0.0.1:8080/readyz
```

Require HTTP 200 for health and 204 for readiness. `/healthz` is process metadata;
`/readyz` verifies the database. Neither proves that a Provider or research plan
is enabled. Verify the installed CLI version and its private mTLS connection;
where there are existing authorized jobs, read status/events without executing
them. With no existing job or execution plan, record that limitation and verify
execution is denied rather than creating a synthetic production research job.
Unknown job IDs return authorization denial (CLI exit 3), intentionally hiding
job existence. That response alone does not prove owner-scoped observation;
the installed synthetic workflow tests provide that behavior's acceptance.

## Recovery without deleting evidence

If migration fails before COMMIT, preserve its diagnostics and backup, verify
versions remain at the pre-upgrade snapshot, and keep application writers stopped.
The migration transaction rolls back; no down-migration is needed.

If application startup/readiness fails after COMMIT, stop the service and repair
configuration or deploy a corrected migration-aware binary. Retain the database,
new index, original artifacts, receipts, audit history and reservations. For
an existing deployment, repoint `current` only to a binary verified compatible
with all applied migrations. For this first install, stopping the new service
is the application rollback; the existing database can remain upgraded.

Disable new execution by omitting the optional executor configuration while
retaining owner identities and the same database in a migration-aware stop-only
deployment. Do not reset dispatch states, budgets, retries or receipt keys. Do
not restore an old dump over production after new writes: first preserve the
new evidence and design a separate verified recovery/import. The backup is a
recovery source, not authorization to erase newer immutable history.
