mod deployment;
mod holdout;
mod process;
mod tls;

use std::os::unix::fs::PermissionsExt;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use loop_protocol::wire::jobs::v1::*;
use loop_protocol::wire::v1::{JobId, JobRecord, Sha256Digest};
use prost::Message;
use sha2::{Digest, Sha256};
use tonic::{Code, Request};

use super::{Fixture, model, support};
use crate::runtime::{
    ArtifactBroker, DataPin, Identity, JobPin, Role, RuntimeAuthority, RuntimeService,
};
use crate::store::{JobRepository, PgJobStore};
use support::{NOW, actor, context};

struct Running {
    fixture: Fixture,
    clock: Arc<support::FixtureClock>,
    store: PgJobStore,
    tls: tls::Credentials,
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<crate::store::StoreResult<()>>,
    views: std::path::PathBuf,
    authority: Arc<RuntimeAuthority>,
    broker: Arc<ArtifactBroker>,
}

impl Running {
    async fn start(role: Role) -> Self {
        Self::start_with(role, false).await
    }

    async fn start_with(role: Role, protected_job: bool) -> Self {
        let mut fixture = Fixture::new();
        let selection = fixture
            .job
            .specification
            .protocol_selection
            .as_mut()
            .unwrap();
        selection.schema_descriptor_sha256 = Some(Sha256Digest {
            value: Sha256::digest(loop_protocol::FILE_DESCRIPTOR_SET).to_vec(),
        });
        selection.selection_sha256 = Some(Sha256Digest {
            value: loop_protocol::job::protocol_selection_sha256(selection)
                .unwrap()
                .to_vec(),
        });
        let clock = Arc::new(support::FixtureClock(AtomicI64::new(NOW)));
        let store = if protected_job {
            holdout::seed(&mut fixture, clock.clone()).await
        } else {
            let store = fixture.open(clock.clone(), fixture.policy().await).await;
            store.submit(fixture.job.clone()).await.unwrap();
            store
        };
        let tls = tls::Credentials::new(fixture.directory.path());
        let actor = actor();
        let identity = Identity {
            actor_id: actor.actor_id.as_ref().unwrap().value.clone(),
            subject: actor.authenticated_subject,
            display_name: actor.display_name,
            role,
            certificate_sha256: vec![tls.client_digest()],
            not_before_ms: NOW - 1000,
            expires_at_ms: NOW + 3_600_000,
            run_ids: vec![
                fixture
                    .job
                    .specification
                    .run_id
                    .as_ref()
                    .unwrap()
                    .value
                    .clone(),
            ],
        };
        let authority = Arc::new(
            RuntimeAuthority::new(
                vec![identity],
                vec![JobPin {
                    job_id: fixture
                        .job
                        .specification
                        .job_id
                        .as_ref()
                        .unwrap()
                        .value
                        .clone(),
                    specification_sha256: format!(
                        "sha256:{:x}",
                        Sha256::digest(fixture.job.specification.encode_to_vec())
                    ),
                }],
                clock.clone(),
            )
            .unwrap(),
        );
        let protected = fixture.directory.path().join("protected");
        let views = fixture.directory.path().join("views");
        let development = if protected_job {
            std::fs::rename(&fixture.root, &protected).unwrap();
            let development = fixture.root.clone();
            std::fs::create_dir(&development).unwrap();
            fixture.root = protected.clone();
            development
        } else {
            std::fs::create_dir(&protected).unwrap();
            fixture.root.clone()
        };
        std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(&views).unwrap();
        std::fs::set_permissions(&views, std::fs::Permissions::from_mode(0o700)).unwrap();
        let broker = Arc::new(
            ArtifactBroker::open(
                &development,
                &protected,
                &views,
                vec![DataPin {
                    job_id: fixture
                        .job
                        .specification
                        .job_id
                        .as_ref()
                        .unwrap()
                        .value
                        .clone(),
                    manifest: fixture.context.data.clone(),
                    protected: protected_job,
                }],
            )
            .unwrap(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let service = RuntimeService::new(store.clone(), authority.clone(), broker.clone());
        let task = tokio::spawn(crate::runtime::serve(
            service,
            listener,
            tls.server(),
            std::future::pending(),
        ));
        Self {
            fixture,
            clock,
            store,
            tls,
            address,
            task,
            views,
            authority,
            broker,
        }
    }

    async fn restart(&mut self) {
        self.task.abort();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        self.address = listener.local_addr().unwrap();
        let service = RuntimeService::new(
            self.store.clone(),
            self.authority.clone(),
            self.broker.clone(),
        );
        self.task = tokio::spawn(crate::runtime::serve(
            service,
            listener,
            self.tls.server(),
            std::future::pending(),
        ));
    }

    fn id(&self) -> Option<JobId> {
        self.fixture.job.specification.job_id.clone()
    }

    async fn client(&self) -> job_service_client::JobServiceClient<tonic::transport::Channel> {
        self.tls.client(self.address, true).await.unwrap()
    }

    async fn acquire(&self) -> JobRecord {
        self.acquire_response().await.into_inner().job.unwrap()
    }

    async fn acquire_response(&self) -> tonic::Response<AcquireJobLeaseResponse> {
        self.client()
            .await
            .acquire_job_lease(AcquireJobLeaseRequest {
                context: Some(context("tls-acquire")),
                job_id: self.id(),
                expected_revision: 1,
                requested_duration: Some(prost_types::Duration {
                    seconds: 60,
                    nanos: 0,
                }),
            })
            .await
            .unwrap()
    }

    fn data_request(&self, job: &JobRecord) -> PrepareJobArtifactsRequest {
        PrepareJobArtifactsRequest {
            context: Some(context("tls-data")),
            job_id: self.id(),
            lease_id: job.active_lease.as_ref().unwrap().lease_id.clone(),
            expected_revision: job.revision,
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
        if let Ok(entries) = std::fs::read_dir(&self.views) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    let _ = std::fs::set_permissions(
                        entry.path(),
                        std::fs::Permissions::from_mode(0o700),
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn mtls_reads_only_pinned_jobs() {
    let running = Running::start(Role::Research).await;
    let job = running
        .client()
        .await
        .get_job(GetJobRequest {
            job_id: running.id(),
        })
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    assert_eq!(
        job.specification,
        Some(running.fixture.job.specification.clone())
    );
}

#[tokio::test]
async fn mtls_requires_a_client_certificate() {
    let running = Running::start(Role::Research).await;
    if let Ok(mut client) = running.tls.client(running.address, false).await {
        assert!(
            client
                .get_job(GetJobRequest {
                    job_id: running.id()
                })
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn an_unregistered_ca_signed_client_is_denied() {
    let running = Running::start(Role::Research).await;
    let mut client = running
        .tls
        .named_client(running.address, Some("unknown"))
        .await
        .unwrap();
    assert_eq!(
        client
            .get_job(GetJobRequest {
                job_id: running.id()
            })
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
}

#[tokio::test]
async fn a_different_client_ca_is_rejected() {
    let running = Running::start(Role::Research).await;
    let directory = tempfile::Builder::new()
        .prefix("loop-runtime-rogue-ca-")
        .tempdir()
        .unwrap();
    let rogue = tls::Credentials::new(directory.path());
    if let Ok(mut client) = rogue.client(running.address, true).await {
        assert!(
            client
                .get_job(GetJobRequest {
                    job_id: running.id()
                })
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn provider_cannot_read_research_jobs() {
    let running = Running::start(Role::Provider).await;
    let error = running
        .client()
        .await
        .get_job(GetJobRequest {
            job_id: running.id(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
}

#[tokio::test]
async fn discovery_cannot_present_a_capability() {
    let running = Running::start(Role::Discovery).await;
    let mut request = Request::new(GetJobRequest {
        job_id: running.id(),
    });
    request.metadata_mut().insert_bin(
        "loop-holdout-capability-bin",
        tonic::metadata::MetadataValue::from_bytes(&[7; 32]),
    );
    assert_eq!(
        running
            .client()
            .await
            .get_job(request)
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
}

#[tokio::test]
async fn forwarded_identity_is_rejected() {
    let running = Running::start(Role::Research).await;
    let mut request = Request::new(GetJobRequest {
        job_id: running.id(),
    });
    request
        .metadata_mut()
        .insert("x-loop-actor", "operator".parse().unwrap());
    assert_eq!(
        running
            .client()
            .await
            .get_job(request)
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
}

#[tokio::test]
async fn body_actor_cannot_impersonate_a_peer() {
    let running = Running::start(Role::Research).await;
    let mut context = context("tls-spoof");
    context.actor.as_mut().unwrap().authenticated_subject = "service:other".to_owned();
    let error = running
        .client()
        .await
        .acquire_job_lease(AcquireJobLeaseRequest {
            context: Some(context),
            job_id: running.id(),
            expected_revision: 1,
            requested_duration: Some(prost_types::Duration {
                seconds: 60,
                nanos: 0,
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(
        running
            .store
            .get(&running.id().unwrap().value)
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}

#[tokio::test]
async fn authenticated_data_view_is_immutable_and_audited() {
    let running = Running::start(Role::Research).await;
    let job = running.acquire().await;
    let request = running.data_request(&job);
    let first = running
        .client()
        .await
        .prepare_job_artifacts(request.clone())
        .await
        .unwrap()
        .into_inner();
    let repeated = running
        .client()
        .await
        .prepare_job_artifacts(request)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first, repeated);
    let view = running.views.join(&first.view_id);
    let data: model::Dataset = serde_json::from_slice(
        &std::fs::read(running.fixture.path(&running.fixture.context.data)).unwrap(),
    )
    .unwrap();
    for artifact in &data.snapshots[0].artifacts {
        let bytes = std::fs::read(view.join(&artifact.object.sha256[7..])).unwrap();
        assert_eq!(
            bytes,
            std::fs::read(running.fixture.path(&artifact.object)).unwrap()
        );
        assert_eq!(
            std::fs::metadata(view.join(&artifact.object.sha256[7..]))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o444
        );
    }
    assert_eq!(running.store.audit_events(0, 20).await.unwrap().len(), 3);
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 1);
}

#[tokio::test]
async fn an_expired_lease_cannot_read_data() {
    let running = Running::start(Role::Research).await;
    let job = running.acquire().await;
    running.clock.0.store(NOW + 60_000, Ordering::SeqCst);
    assert_eq!(
        running
            .client()
            .await
            .prepare_job_artifacts(running.data_request(&job))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 0);
}

#[tokio::test]
async fn a_changed_file_does_not_publish_a_view() {
    let running = Running::start(Role::Research).await;
    let job = running.acquire().await;
    let data: model::Dataset = serde_json::from_slice(
        &std::fs::read(running.fixture.path(&running.fixture.context.data)).unwrap(),
    )
    .unwrap();
    std::fs::write(
        running.fixture.path(&data.snapshots[0].artifacts[0].object),
        b"corrupt",
    )
    .unwrap();
    assert_eq!(
        running
            .client()
            .await
            .prepare_job_artifacts(running.data_request(&job))
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 0);
    assert_eq!(running.store.audit_events(0, 20).await.unwrap().len(), 2);
}

fn authorized<T>(
    value: T,
    token: &tonic::metadata::MetadataValue<tonic::metadata::Binary>,
) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert_bin("loop-holdout-capability-bin", token.clone());
    request
}

#[tokio::test]
async fn protected_view_requires_the_current_capability() {
    let running = Running::start_with(Role::HoldoutWorker, true).await;
    let acquired = running.acquire_response().await;
    let token = acquired
        .metadata()
        .get_bin("loop-holdout-capability-bin")
        .unwrap()
        .clone();
    let request = running.data_request(acquired.get_ref().job.as_ref().unwrap());
    let mut client = running.client().await;
    assert_eq!(
        client
            .prepare_job_artifacts(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    let view = client
        .prepare_job_artifacts(authorized(request, &token))
        .await
        .unwrap()
        .into_inner();
    assert!(!view.artifacts.is_empty());
    assert_eq!(
        std::fs::read_dir(running.views.join(view.view_id))
            .unwrap()
            .count(),
        view.artifacts.len()
    );
    let events = running.store.audit_events(0, 100).await.unwrap();
    loop_core::audit::verify_audit_chain(&events).unwrap();
    let encoded = token.as_encoded_bytes();
    assert!(
        !format!("{events:?}")
            .as_bytes()
            .windows(encoded.len())
            .any(|window| window == encoded)
    );
}

#[tokio::test]
async fn development_worker_cannot_acquire_protected_work() {
    let running = Running::start_with(Role::Research, true).await;
    let error = running
        .client()
        .await
        .get_job(GetJobRequest {
            job_id: running.id(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
}

#[tokio::test]
async fn restart_does_not_accept_an_old_capability() {
    let mut running = Running::start_with(Role::HoldoutWorker, true).await;
    let acquired = running.acquire_response().await;
    let token = acquired
        .metadata()
        .get_bin("loop-holdout-capability-bin")
        .unwrap()
        .clone();
    let request = running.data_request(acquired.get_ref().job.as_ref().unwrap());
    running.restart().await;
    let error = running
        .client()
        .await
        .prepare_job_artifacts(authorized(request.clone(), &token))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    let repeated = running.acquire_response().await;
    assert_eq!(repeated.get_ref(), acquired.get_ref());
    let current = repeated
        .metadata()
        .get_bin("loop-holdout-capability-bin")
        .unwrap();
    assert_ne!(current, &token);
    running
        .client()
        .await
        .prepare_job_artifacts(authorized(request, current))
        .await
        .unwrap();
}

#[tokio::test]
async fn expired_acquisition_does_not_renew_authority() {
    let running = Running::start_with(Role::HoldoutWorker, true).await;
    let acquired = running.acquire_response().await;
    running.clock.0.store(NOW + 60_000, Ordering::SeqCst);
    let mut request = AcquireJobLeaseRequest {
        context: Some(context("tls-acquire")),
        job_id: running.id(),
        expected_revision: 1,
        requested_duration: Some(prost_types::Duration {
            seconds: 60,
            nanos: 0,
        }),
    };
    request.context.as_mut().unwrap().requested_at = Some(support::timestamp(NOW + 60_000));
    let replay = running
        .client()
        .await
        .acquire_job_lease(request)
        .await
        .unwrap();
    assert_eq!(replay.get_ref(), acquired.get_ref());
    assert!(
        replay
            .metadata()
            .get_bin("loop-holdout-capability-bin")
            .is_none()
    );
}

#[tokio::test]
async fn duplicate_capability_headers_are_denied() {
    let running = Running::start_with(Role::HoldoutWorker, true).await;
    let acquired = running.acquire_response().await;
    let token = acquired
        .metadata()
        .get_bin("loop-holdout-capability-bin")
        .unwrap();
    let mut request = authorized(
        running.data_request(acquired.get_ref().job.as_ref().unwrap()),
        token,
    );
    request
        .metadata_mut()
        .append_bin("loop-holdout-capability-bin", token.clone());
    assert_eq!(
        running
            .client()
            .await
            .prepare_job_artifacts(request)
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
}

#[tokio::test]
async fn an_unknown_job_does_not_reveal_existence() {
    let running = Running::start(Role::Provider).await;
    let unknown = Some(JobId {
        value: "job.absent".to_owned(),
    });
    let mut client = running.client().await;
    let known = client
        .get_job(GetJobRequest {
            job_id: running.id(),
        })
        .await
        .unwrap_err();
    let unknown = client
        .get_job(GetJobRequest { job_id: unknown })
        .await
        .unwrap_err();
    assert_eq!(known.code(), unknown.code());
    assert_eq!(known.message(), unknown.message());
    assert_eq!(known.details(), unknown.details());
}

#[tokio::test]
async fn identity_expiry_applies_to_existing_connections() {
    let running = Running::start(Role::Research).await;
    let mut client = running.client().await;
    client
        .get_job(GetJobRequest {
            job_id: running.id(),
        })
        .await
        .unwrap();
    running.clock.0.store(NOW + 3_600_000, Ordering::SeqCst);
    assert_eq!(
        client
            .get_job(GetJobRequest {
                job_id: running.id()
            })
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
}

#[tokio::test]
async fn regressed_runtime_clock_denies_reads() {
    let running = Running::start(Role::Research).await;
    let mut client = running.client().await;
    client
        .get_job(GetJobRequest {
            job_id: running.id(),
        })
        .await
        .unwrap();
    running.clock.0.store(NOW - 1, Ordering::SeqCst);
    assert_eq!(
        client
            .get_job(GetJobRequest {
                job_id: running.id()
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
}

#[tokio::test]
async fn a_stale_request_does_not_publish_data() {
    let running = Running::start(Role::Research).await;
    let job = running.acquire().await;
    running.clock.0.store(NOW + 30_000, Ordering::SeqCst);
    assert_eq!(
        running
            .client()
            .await
            .prepare_job_artifacts(running.data_request(&job))
            .await
            .unwrap_err()
            .code(),
        Code::DeadlineExceeded
    );
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 0);
    assert_eq!(running.store.audit_events(0, 20).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_tampered_view_is_not_overwritten() {
    let running = Running::start(Role::Research).await;
    let job = running.acquire().await;
    let request = running.data_request(&job);
    let view = running
        .client()
        .await
        .prepare_job_artifacts(request.clone())
        .await
        .unwrap()
        .into_inner();
    let path = running
        .views
        .join(view.view_id)
        .join(&view.artifacts[0].artifact_id.as_ref().unwrap().value[7..]);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, b"tampered").unwrap();
    assert_eq!(
        running
            .client()
            .await
            .prepare_job_artifacts(request)
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(std::fs::read(path).unwrap(), b"tampered");
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 1);
    assert_eq!(running.store.audit_events(0, 20).await.unwrap().len(), 3);
}

#[tokio::test]
async fn data_receipt_projection_is_verified() {
    use sqlx::Executor;
    let running = Running::start(Role::Research).await;
    let job = running.acquire().await;
    let request = running.data_request(&job);
    running
        .client()
        .await
        .prepare_job_artifacts(request.clone())
        .await
        .unwrap();
    let mut db = support::connection(&running.fixture.directory).await;
    db.execute("ALTER TABLE command_receipts DISABLE TRIGGER command_receipts_no_update")
        .await
        .unwrap();
    db.execute(
        "UPDATE command_receipts SET request_id='tampered' WHERE operation='loop.jobs.artifacts'",
    )
    .await
    .unwrap();
    assert_eq!(
        running
            .client()
            .await
            .prepare_job_artifacts(request)
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(running.store.audit_events(0, 20).await.unwrap().len(), 3);
}

#[tokio::test]
async fn audit_failure_rolls_back_data_acceptance() {
    use sqlx::Executor;
    let running = Running::start(Role::Research).await;
    let job = running.acquire().await;
    let request = running.data_request(&job);
    let mut db = support::connection(&running.fixture.directory).await;
    db.execute("CREATE TRIGGER injected_data_failure BEFORE INSERT ON audit_events FOR EACH ROW EXECUTE FUNCTION reject_immutable_change()").await.unwrap();
    assert_eq!(
        running
            .client()
            .await
            .prepare_job_artifacts(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 0);
    assert_eq!(running.store.audit_events(0, 20).await.unwrap().len(), 2);
    db.execute("DROP TRIGGER injected_data_failure ON audit_events")
        .await
        .unwrap();
    running
        .client()
        .await
        .prepare_job_artifacts(request)
        .await
        .unwrap();
    assert_eq!(running.store.audit_events(0, 20).await.unwrap().len(), 3);
}

#[tokio::test]
async fn a_cancelled_job_cannot_replay_data_access() {
    let running = Running::start(Role::Research).await;
    let job = running.acquire().await;
    let request = running.data_request(&job);
    running
        .client()
        .await
        .prepare_job_artifacts(request.clone())
        .await
        .unwrap();
    running
        .store
        .mutate(
            &actor(),
            crate::store::JobMutation::Cancel(CancelJobRequest {
                context: Some(context("cancel-data")),
                job_id: running.id(),
                expected_revision: job.revision,
                reason: "test lease cancellation".to_owned(),
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        running
            .client()
            .await
            .prepare_job_artifacts(request)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
}
