import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";

// Administrative deployment via SSH/peer authentication. This emits no secrets.
// SQLx 0.8.6 uses SHA-384 of each exact migration file for its migration checksum.
const directory = new URL("../migrations/postgres/", import.meta.url);
const migrations = readdirSync(directory)
  .filter((name) => /^\d{4}_[a-z_]+\.sql$/.test(name))
  .sort();
if (migrations.length === 0) throw new Error("no PostgreSQL migrations found");
const literal = (text) => `'${text.replaceAll("'", "''")}'`;
const statements = [
  "\\set ON_ERROR_STOP on",
  "SELECT current_database() = 'loop_engine' AS target_ok \\gset",
  "\\if :target_ok",
  "BEGIN;",
  "SET LOCAL ROLE loop_engine_owner;",
  "SET LOCAL search_path = public, pg_catalog;",
  "SET LOCAL statement_timeout = '30s';",
  "SET LOCAL lock_timeout = '5s';",
  "SELECT pg_advisory_xact_lock(hashtextextended('loop.migrations.public', 0));",
  `CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY, description TEXT NOT NULL,
    installed_on TIMESTAMPTZ NOT NULL DEFAULT now(), success BOOLEAN NOT NULL,
    checksum BYTEA NOT NULL, execution_time BIGINT NOT NULL
  );`,
];
for (const name of migrations) {
  const version = Number(name.slice(0, 4));
  const source = readFileSync(new URL(name, directory), "utf8");
  const checksum = createHash("sha384").update(source, "utf8").digest("hex");
  const description = name.slice(5, -4).replaceAll("_", " ");
  statements.push(
    `DO $guard$ BEGIN
      IF EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = ${version}
        AND (checksum != decode('${checksum}', 'hex') OR NOT success)) THEN
        RAISE EXCEPTION 'migration ${version} checksum mismatch';
      END IF;
    END $guard$;`,
    `SELECT NOT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version = ${version}) AS apply_${version} \\gset`,
    `\\if :apply_${version}`,
    source,
    `INSERT INTO _sqlx_migrations(version, description, success, checksum, execution_time)
      VALUES (${version}, ${literal(description)}, true, decode('${checksum}', 'hex'), 0);`,
    "\\endif",
  );
}
statements.push(
  "INSERT INTO store_metadata VALUES (1, 'ledger.loopd', 0) ON CONFLICT(singleton) DO NOTHING;",
  "GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA public TO loop_engine_app;",
  "GRANT UPDATE ON store_metadata, jobs, holdout_periods, holdout_grants, perturbation_states, factor_states TO loop_engine_app;",
  "REVOKE INSERT, UPDATE, DELETE ON _sqlx_migrations FROM loop_engine_app;",
  "COMMIT;",
  "\\else",
  "\\echo Refusing a database other than loop_engine",
  "\\quit 3",
  "\\endif",
);
process.stdout.write(statements.join("\n") + "\n");
