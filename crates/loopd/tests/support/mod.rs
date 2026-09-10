#![allow(dead_code)]

pub mod approval;
pub mod backtest;
pub mod batch;
pub mod grant;
pub mod holdout;
pub mod rejection;
pub mod research;

use std::path::Path;
use std::str::FromStr;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use loop_protocol::job::protocol_selection_sha256;
use loop_protocol::negotiation::{ProtocolBuildIdentity, validate_protocol_selection_availability};
use loop_protocol::wire::v1::*;
use loopd::store::{
    AdmissionPolicy, Clock, PgJobStore, StoreError, StoreOptions, StoreResult, SubmitJob,
};
use prost_types::{Duration, Timestamp};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, postgres::PgConnectOptions};
use tempfile::TempDir;

pub const NOW: i64 = 1_788_761_610_000;

pub struct FixtureClock(pub AtomicI64);

impl Clock for FixtureClock {
    fn now_millis(&self) -> StoreResult<i64> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

pub struct FixtureAdmission;

impl AdmissionPolicy for FixtureAdmission {
    fn authorize_job_command(&self, _: &str, actor: &Actor, _: &JobRecord) -> StoreResult<()> {
        if actor.authenticated_subject == "service:fixture" {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }

    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if job.kind != JobKind::Report as i32 {
            return Err(StoreError::AdmissionDenied);
        }
        let Some(job_specification::Input::Artifact(input)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        if input
            .input
            .as_ref()
            .and_then(|artifact| artifact.sha256.as_ref())
            != Some(&digest(24))
        {
            return Err(StoreError::AdmissionDenied);
        }
        validate_protocol_selection_availability(
            job.protocol_selection
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?,
            &protocol_info(),
            &[ProtocolBuildIdentity {
                build_version: "fixture.1".to_owned(),
                build_sha256: [23; 32],
            }],
            &[[22; 32]],
            "loop.v1",
            &["jobs.envelope.v1", "jobs.kind-input.v1"],
        )
        .map_err(|_| StoreError::AdmissionDenied)
    }
}

pub fn options(path: &Path, clock: Arc<FixtureClock>) -> StoreOptions {
    let mut options = base_options(path);
    options.clock = clock;
    options.admission = Arc::new(FixtureAdmission);
    options
}

pub fn test_url() -> String {
    let url = std::env::var("LOOP_TEST_POSTGRES_URL").unwrap_or_else(|_| {
        "postgresql://loop_engine_test:loop_engine_test_only@127.0.0.1:15433/loop_engine_test?sslmode=require".to_owned()
    });
    let parsed = PgConnectOptions::from_str(&url).expect("test PostgreSQL URL");
    assert_eq!(
        parsed.get_database(),
        Some("loop_engine_test"),
        "refusing a non-test database"
    );
    assert_eq!(
        parsed.get_username(),
        "loop_engine_test",
        "refusing a production principal"
    );
    url
}

pub fn schema(path: &Path) -> String {
    let digest = format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes()));
    format!("test_{}", &digest[..32])
}

pub fn base_options(path: &Path) -> StoreOptions {
    let mut options = StoreOptions::new(&test_url()).unwrap();
    options.schema = schema(path);
    options.apply_migrations = true;
    options
}

pub async fn fixture() -> (TempDir, PgJobStore, Arc<FixtureClock>) {
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixtureClock(AtomicI64::new(NOW)));
    let store = PgJobStore::open(options(&directory.path().join("state"), clock.clone()))
        .await
        .unwrap();
    (directory, store, clock)
}

pub async fn connection(directory: &TempDir) -> PgConnection {
    let search_path = schema(&directory.path().join("state"));
    PgConnection::connect_with(&PgConnectOptions::from_str(&test_url()).unwrap().options([
        ("search_path", search_path.as_str()),
        ("lock_timeout", "5000"),
        ("statement_timeout", "30000"),
    ]))
    .await
    .unwrap()
}

pub fn command(index: u32) -> SubmitJob {
    let mut selection = ProtocolSelectionSnapshot {
        selected_package: "loop.v1".to_owned(),
        enabled_features: protocol_info().features,
        effective_limits: protocol_info().limits,
        server_build_version: "fixture.1".to_owned(),
        server_build_sha256: Some(digest(21)),
        schema_descriptor_sha256: Some(digest(22)),
        selection_sha256: None,
        selected_at: Some(timestamp(NOW - 2_000)),
        client_build_version: "fixture.1".to_owned(),
        client_build_sha256: Some(digest(23)),
    };
    selection.selection_sha256 = Some(Sha256Digest {
        value: protocol_selection_sha256(&selection).unwrap().to_vec(),
    });
    SubmitJob {
        request_id: format!("request.{index}"),
        specification: JobSpecification {
            job_id: Some(JobId {
                value: format!("job.{index}"),
            }),
            run_id: Some(RunId {
                value: "run.fixture".to_owned(),
            }),
            kind: JobKind::Report as i32,
            input: Some(job_specification::Input::Artifact(ArtifactJobInput {
                input: Some(artifact()),
                policy: Some(PolicyReference {
                    policy_id: Some(PolicyId {
                        value: "policy.fixture".to_owned(),
                    }),
                    revision: "1".to_owned(),
                    sha256: Some(digest(6)),
                }),
                budget: Some(JobBudget {
                    maximum_steps: 40,
                    maximum_input_tokens: 100_000,
                    maximum_output_tokens: 20_000,
                    maximum_cost: Some(Money {
                        amount: Some(ExactDecimal {
                            value: "10".to_owned(),
                        }),
                        currency_code: "USD".to_owned(),
                    }),
                    maximum_wall_time: Some(Duration {
                        seconds: 3_600,
                        nanos: 0,
                    }),
                }),
            })),
            submitted_at: Some(timestamp(NOW - 1_000)),
            submitted_by: Some(Actor {
                actor_id: Some(ActorId {
                    value: "service.fixture".to_owned(),
                }),
                kind: ActorKind::Service as i32,
                display_name: "Fixture submitter".to_owned(),
                authenticated_subject: "service:fixture".to_owned(),
            }),
            idempotency_key: Some(IdempotencyKey {
                value: format!("submit.{index}"),
            }),
            correlation_id: Some(CorrelationId {
                value: "corr.fixture".to_owned(),
            }),
            causation_id: Some(CausationId {
                value: "cause.fixture".to_owned(),
            }),
            protocol_selection: Some(selection),
        },
    }
}

pub fn protocol_info() -> ProtocolInfo {
    ProtocolInfo {
        supported_packages: vec!["loop.v1".to_owned()],
        features: vec![
            "jobs.envelope.v1".to_owned(),
            "jobs.kind-input.v1".to_owned(),
            "jobs.prelease-terminal.v1".to_owned(),
        ],
        limits: Some(ProtocolLimits {
            maximum_unary_bytes: 4_194_304,
            maximum_stream_event_bytes: 1_048_576,
            maximum_canonical_ast_bytes: 262_144,
            maximum_ast_nodes: 4_096,
            maximum_ast_depth: 64,
            maximum_page_records: 500,
            maximum_identity_bytes: 128,
            maximum_artifact_uri_bytes: 2_048,
        }),
        build_version: "fixture.1".to_owned(),
        build_sha256: Some(digest(21)),
    }
}

pub fn artifact() -> ArtifactRef {
    let hex = "18".repeat(32);
    ArtifactRef {
        artifact_id: Some(ArtifactId {
            value: format!("sha256:{hex}"),
        }),
        uri: format!("artifact://sha256/{hex}"),
        sha256: Some(digest(24)),
        schema: Some(ArtifactSchemaReference {
            name: "loop.report".to_owned(),
            version: 1,
            schema_sha256: Some(digest(25)),
        }),
        media_type: "application/json".to_owned(),
        byte_size: 1,
        created_at: Some(timestamp(NOW - 5_000)),
        ..Default::default()
    }
}

pub fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest {
        value: vec![byte; 32],
    }
}
pub fn timestamp(millis: i64) -> Timestamp {
    Timestamp {
        seconds: millis / 1_000,
        nanos: ((millis % 1_000) * 1_000_000) as i32,
    }
}

pub fn actor() -> Actor {
    command(1).specification.submitted_by.unwrap()
}

pub fn context(key: &str) -> CommandContext {
    CommandContext {
        request_id: Some(RequestId {
            value: format!("request.{key}"),
        }),
        correlation_id: Some(CorrelationId {
            value: "corr.fixture".to_owned(),
        }),
        causation_id: Some(CausationId {
            value: "cause.fixture".to_owned(),
        }),
        idempotency_key: Some(IdempotencyKey {
            value: key.to_owned(),
        }),
        actor: Some(actor()),
        requested_at: Some(timestamp(NOW)),
    }
}
