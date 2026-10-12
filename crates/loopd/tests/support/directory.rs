use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::{Connection, PgConnection, postgres::PgConnectOptions};

/// Owns one temporary directory and its exact `state` PostgreSQL namespace.
///
/// Subprocesses borrow the directory path; they never acquire this cleanup guard.
/// Drop also runs during assertion unwinding. A killed test process is covered
/// by the suite's disposable-database teardown, not by scanning other schemas.
pub struct TestDirectory {
    directory: tempfile::TempDir,
    url: String,
}

impl Default for TestDirectory {
    fn default() -> Self {
        Self::new()
    }
}

impl TestDirectory {
    pub fn new() -> Self {
        Self::from_temp(tempfile::tempdir().unwrap())
    }

    pub fn from_temp(directory: tempfile::TempDir) -> Self {
        Self {
            directory,
            url: super::test_url(),
        }
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let namespace = super::schema(&self.path().join("state"));
        let url = self.url.clone();
        // Drop may run inside a current-thread Tokio test. A separate runtime
        // avoids nested block_on and keeps database waits bounded during unwind.
        let result = std::thread::Builder::new()
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?;
                let result = runtime.block_on(async {
                    tokio::time::timeout(Duration::from_secs(15), cleanup_schema(&url, &namespace))
                        .await
                        .map_err(|_| "fixture cleanup deadline".to_owned())?
                });
                runtime.shutdown_timeout(Duration::from_secs(1));
                result
            })
            .map_err(|error| error.to_string())
            .and_then(|thread| {
                thread
                    .join()
                    .unwrap_or_else(|_| Err("fixture cleanup thread panicked".to_owned()))
            });
        if let Err(error) = result {
            if std::thread::panicking() {
                eprintln!("fixture schema cleanup failed during unwind: {error}");
            } else {
                panic!("fixture schema cleanup failed: {error}");
            }
        }
    }
}

async fn cleanup_schema(url: &str, namespace: &str) -> Result<(), String> {
    let options = PgConnectOptions::from_str(url)
        .map_err(|error| error.to_string())?
        .options([
            ("search_path", "pg_catalog"),
            ("lock_timeout", "5000"),
            ("statement_timeout", "10000"),
        ]);
    let mut connection = PgConnection::connect_with(&options)
        .await
        .map_err(|error| error.to_string())?;
    // A dropped SQLx transaction may still need its original current-thread
    // runtime to flush rollback. Drop cannot drive that runtime while joining
    // this cleanup thread. Terminate only clients that actually hold an object
    // lock in this guard's namespace, under this dedicated test DB/principal.
    // A false termination result can mean the client exited naturally; only
    // the subsequent successful DROP establishes that cleanup is complete.
    sqlx::query(
        "WITH target AS (
            SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = $1
         ), owners AS (
            SELECT DISTINCT activity.pid
            FROM pg_catalog.pg_stat_activity AS activity
            JOIN pg_catalog.pg_locks AS held ON held.pid = activity.pid
            CROSS JOIN target
            WHERE activity.datid = (
                SELECT oid FROM pg_catalog.pg_database
                WHERE datname = pg_catalog.current_database()
            )
              AND activity.usesysid = (CURRENT_USER::pg_catalog.regrole)::oid
              AND activity.backend_type = 'client backend'
              AND activity.pid <> pg_catalog.pg_backend_pid()
              AND held.database = activity.datid
              AND held.granted
              AND (
                (held.locktype = 'relation' AND EXISTS (
                    SELECT FROM pg_catalog.pg_class AS relation
                    WHERE relation.oid = held.relation
                      AND relation.relnamespace = target.oid
                ))
                OR (held.locktype = 'object'
                    AND held.classid = 'pg_catalog.pg_namespace'::pg_catalog.regclass
                    AND held.objid = target.oid)
              )
         )
         SELECT pg_catalog.pg_terminate_backend(pid) FROM owners",
    )
    .bind(namespace)
    .execute(&mut connection)
    .await
    .map_err(|error| error.to_string())?;
    // namespace is derived only from this guard's owned random TempDir, never
    // from caller-supplied SQL or a broad test_* prefix.
    sqlx::query(&format!("DROP SCHEMA IF EXISTS \"{namespace}\" CASCADE"))
        .execute(&mut connection)
        .await
        .map_err(|error| error.to_string())?;
    connection.close().await.map_err(|error| error.to_string())
}
