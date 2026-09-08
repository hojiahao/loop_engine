mod support;

use loopd::store::{PgJobStore, StoreError, StoreOptions};
use sqlx::Connection;
use sqlx::migrate::Migrate;
use support::*;

#[test]
fn requires_explicit_tls() {
    for mode in ["require", "verify-ca", "verify-full"] {
        let config = StoreOptions::new(&format!(
            "postgresql://app:fixture@localhost/db?sslmode={mode}"
        ))
        .unwrap();
        assert!(!config.apply_migrations);
    }
    for mode in ["disable", "allow", "prefer", "unknown"] {
        assert!(
            StoreOptions::new(&format!(
                "postgresql://app:fixture@localhost/db?sslmode={mode}"
            ))
            .is_err()
        );
    }
    assert!(StoreOptions::new("postgresql://app:fixture@localhost/db").is_err());
}

#[test]
fn rejects_ambiguous_or_non_postgres_urls() {
    for (index, url) in [
        "https://app:fixture@localhost/db?sslmode=require",
        "postgresql://app:fixture@localhost/?sslmode=require",
        "postgresql://localhost/db?sslmode=require",
        "postgresql://app:fixture@%2Ftmp/db?sslmode=require",
        "postgresql://app:fixture@localhost/db?sslmode=require&sslmode=disable",
        "postgresql://app:fixture@localhost/db?sslmode=require&options=-cfsync=off",
        "postgresql://app:fixture@localhost/db?sslmode=require#ignored",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            StoreOptions::new(url).is_err(),
            "accepted invalid case {index}"
        );
    }
}

#[test]
fn configuration_errors_do_not_echo_secrets() {
    for url in [
        "postgresql://app:sentinel-secret@localhost/db?sslmode=invalid",
        "postgresql://app:fixture@localhost/db?token=sentinel-secret&sslmode=require",
        "postgresql://app:fixture@localhost:sentinel-secret/db?sslmode=require",
    ] {
        let Err(error) = StoreOptions::new(url) else {
            panic!("invalid configuration accepted");
        };
        assert!(!format!("{error:?}: {error}").contains("sentinel-secret"));
    }
}

#[tokio::test]
async fn runtime_does_not_create_missing_schema() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = base_options(&directory.path().join("state"));
    let namespace = config.schema.clone();
    config.apply_migrations = false;
    assert!(PgJobStore::open(config).await.is_err());
    let mut connection = connection(&directory).await;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_namespace WHERE nspname = $1)")
            .bind(namespace)
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert!(!exists);
    connection.close().await.unwrap();
}

#[tokio::test]
async fn runtime_rejects_changed_migration_checksum() {
    let (directory, store, _) = fixture().await;
    let mut connection = connection(&directory).await;
    sqlx::query(
        "UPDATE _sqlx_migrations SET checksum = decode(repeat('00', 48), 'hex') WHERE version = 1",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    assert!(matches!(
        store.verify_configuration().await,
        Err(StoreError::Corrupt(_))
    ));
    connection.close().await.unwrap();
    store.close().await;
    let mut config = base_options(&directory.path().join("state"));
    config.apply_migrations = false;
    assert!(matches!(
        PgJobStore::open(config).await,
        Err(StoreError::Corrupt(_))
    ));
}

#[tokio::test]
async fn schema_names_cannot_inject_sql() {
    for name in [
        "",
        "BadSchema",
        "1invalid",
        "public,evil",
        "x; DROP SCHEMA public",
    ] {
        let mut config = StoreOptions::new(&test_url()).unwrap();
        config.schema = name.to_owned();
        assert!(matches!(
            PgJobStore::open(config).await,
            Err(StoreError::Invalid(_))
        ));
    }
}

#[tokio::test]
async fn independent_schema_has_its_own_migration_lock() {
    let directory = tempfile::tempdir().unwrap();
    let mut other_migrator = connection(&directory).await;
    other_migrator.lock().await.unwrap();
    let store = PgJobStore::open(base_options(&directory.path().join("state")))
        .await
        .unwrap();
    store.verify_configuration().await.unwrap();
    other_migrator.close().await.unwrap();
    store.close().await;
}
