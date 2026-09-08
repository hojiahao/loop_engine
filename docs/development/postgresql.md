# PostgreSQL operations

PostgreSQL is the only runtime metadata backend under ADR 0007. Provider and
research workers never receive database credentials. Market data remains in
immutable artifacts, not PostgreSQL byte columns. The existing production
server runs PostgreSQL 17.11; provisioning does not upgrade or restart it.

## Identities and deployment

- Database: `loop_engine`.
- Runtime login: `loop_engine_app`, without superuser, database/role creation,
  replication, or row-security bypass privileges; connection limit 20.
- Schema owner: `loop_engine_owner`, without login privileges.
- Runtime: schema USAGE; table SELECT/INSERT; UPDATE only on mutable state tables.
  Audit and receipt rows have immutable triggers. Runtime cannot write SQLx
  migration metadata, create schema objects, or delete history.

`infra/postgres/create-production.sql` provisions these names once with a
database administrator. Existing names cause an error: the script does not
silently adopt or overwrite another application's roles or database. Set the
application password using an interactive `psql` `\password` prompt, never SQL
literals, shell arguments, or source control. No administrator password is
needed when the approved host provides local peer authentication through sudo.

For the designated database, generate the forward-only administrative bundle:

```bash
node scripts/postgres-production-bundle.mjs
```

Pipe this output to an administrator's `psql -X -d loop_engine` session on the
approved host. The bundle verifies the target database, locks deployment,
applies pending migrations as the non-login owner, records exact SQLx SHA-384
checksums, and grants runtime privileges in one transaction. Already applied
matching migrations are skipped. Never edit an applied migration: add a new
version. Do not execute this bundle against unrelated databases.
Both the runtime deployment mode and this bundle use the same schema-scoped
advisory lock. SQLx's additional database-wide lock is disabled; administrators
must use these entry points rather than running an uncoordinated SQLx CLI.

## Private connection configuration

Keep remote development behind a host-key-verified SSH tunnel, forwarding
local `127.0.0.1:15432` to the production host's `127.0.0.1:5432`. Do not expose
a new public PostgreSQL listener. Configure the connection with hidden input:

```bash
./scripts/uv.sh run --locked --offline --no-sync python \
  scripts/configure-database.py --host 127.0.0.1 --port 15432 \
  --output var/secrets/loopd-database-url
./scripts/cargo.sh run -p loopd --locked --offline -- --check-database
```

The helper prompts twice, percent-encodes credentials, creates a mode-0600 file,
and refuses to overwrite an existing file. `var/` is Git-ignored. Do not print
this file, include it in a build context, or send its contents to a model.
The Docker build context excludes `var/`; the development container overlays
`/workspace/var` with empty temporary storage rather than exposing host secrets.
Use `LOOPD_DATABASE_URL_FILE` or `--database-url-file` to select another private
file. Never pass the URL or password on a command line.

Runtime startup requires an explicit database and `sslmode=require`,
`verify-ca`, or `verify-full`; lower modes and Unix sockets are rejected. It
verifies that its own session actually uses TLS, synchronous writes, expected
timeouts, UTC, the correct ledger, and the exact migration checksums. The
application performs no DDL during normal startup. `--migrate` is a separate
deployment-only mode requiring owner authority, not an application privilege.

`require` encrypts transport; it does not establish server identity for a
self-signed certificate. The approved SSH host key authenticates the remote hop.
For direct deployments, install a trusted CA/certificate and use `verify-full`.
The shared server's existing HBA rules are not rewritten by this project;
client TLS enforcement is not a claim that all server clients must use TLS.
Rotate credentials through a restricted secret-management workflow, especially
when a password has previously appeared in a chat. No password is reproduced
in this repository or its documentation.

## Isolated integration tests

```bash
bash scripts/postgres-test.sh start
just test
bash scripts/postgres-test.sh stop
```

The test service uses a pinned DaoCloud PostgreSQL 17.11 image, a generated
ephemeral TLS certificate, and a disposable tmpfs volume. It binds only
`127.0.0.1:15433`. Its public fixture login/database `loop_engine_test` and
password `loop_engine_test_only` are intentionally non-production credentials.
Stopping the service discards only this project's disposable test database.

Every fixture creates a unique, strictly validated schema. Worker subprocesses
share only the intended fixture schema. Tests reject a connection unless both
the database and login are exactly `loop_engine_test`, preventing accidental use
of the production `loop_engine` database. `LOOP_TEST_POSTGRES_URL` may redirect
these fixtures to a separately provisioned TLS test service; it is never read
by the production executable. Compose/container CI uses the isolated service
name rather than a host port. Host `just test` starts the service automatically.

Concurrency tests use independent OS processes. Kill/restart tests terminate
clients around transaction boundaries; they are not a claim of storage-device
or entire PostgreSQL-server crash testing. The global ledger-row lock gives
ordered atomic commits at the cost of serializing one ledger's writers. Lock
waits are bounded at five seconds; statements at thirty seconds; connection
acquisition at ten seconds. Additional throughput claims require measurements.

## Current scope

The production schema is infrastructure, not a completed research engine.
Default policies deny commands, mutating transport RPCs remain unavailable,
and no holdout has been registered or unlocked in production. Holdout approval,
grant issuance, batch consumption, data adapters, providers, and research
execution retain their documented later gates in `IMPLEMENTATION_TODO.md`.
