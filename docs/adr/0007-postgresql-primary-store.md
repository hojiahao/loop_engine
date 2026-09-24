# ADR 0007: PostgreSQL as the primary metadata store

- Status: Accepted by owner
- Date: 2026-09-08
- Owner: hojiahao
- Supersedes: ADR 0006's SQLite backend choice, not its transactional invariants

## Decision

PostgreSQL becomes the only primary runtime metadata backend. Development and CI
must exercise PostgreSQL, including independent 2/4/8-process contention and
kill/restart of clients before and after commit. SQLite checkpoints remain
historical evidence; passing their tests does not validate PostgreSQL behavior.
Production records are never used by destructive or fault-injection tests.

The owner's designated production host already runs PostgreSQL 17.11. Provision
only an isolated `loop_engine` database, a non-login `loop_engine_owner` schema
owner, and a `loop_engine_app` login without administrative privileges. Do not
upgrade the shared server, change unrelated databases, or open a public database
listener. Administrative schema migrations are separate from runtime startup.

All application connections require TLS (`sslmode=require` or stronger), bounded
connection/query/lock waits, and verified session settings. `require` encrypts
traffic but does not authenticate a self-signed server certificate. Remote
development therefore uses an authenticated SSH tunnel; a deployment with a
trusted CA should use `verify-full`. Secrets are file references or injected
environment values, never command-line passwords, source files, logs, or receipts.

Deployment migration connections initially resolve only `pg_catalog`. After
acquiring the schema-scoped advisory lock and creating the target namespace,
they set the target search path and verify `current_schema()` before SQLx can
create migration metadata. A nonexistent search-path entry must not silently
redirect unqualified DDL to another schema. Runtime connections receive the
target path separately and never create missing schemas.

Domain models, canonical identities, and role-owned command APIs remain stable.
SQL, migrations, locks, receipt races, schema verification, and backend-specific
tests must be ported and rerun. Updating only a connection URL cannot close this
amendment or Phase 3. Immutable market/research data remains in content-addressed
Parquet, outside the metadata database and service RPC payloads.

## Rollout gates

1. Create dedicated production identities and verify encrypted, least-privilege
   login without touching existing applications.
2. Implement PostgreSQL migrations, configuration, transaction and audit storage;
   runtime startup must not silently fall back to SQLite or claim ready without
   the expected schema and TLS session.
3. Exercise real PostgreSQL contract, concurrency, rollback, cancellation,
   corruption, timeout, and restart tests in an isolated environment.
4. Complete holdout registration, approval, grants, and atomic batch consumption.
5. Pass workspace and clean-container gates, commit and push, and record remote
   CI evidence before closing Phase 3.

The user-provided application password is not reproduced here. It should be
rotated through the secret mechanism before research is exposed to untrusted
networks or users.
