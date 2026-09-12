use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::jobs::v1::{
    self,
    job_service_server::{JobService, JobServiceServer},
};
use loop_protocol::wire::v1::{JobRecord, job_specification};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{
    Request, Response, Status,
    transport::{Server, ServerTlsConfig},
};

use super::authority::{CAPABILITY_HEADER, Principal};
use super::capability::Capabilities;
use super::{ArtifactBroker, RuntimeAuthority};
use crate::store::{JobMutation, JobRepository, PgJobStore, StoreError, StoreResult};

/// Optional mTLS-only job endpoint. The store must use this same deployment
/// authority; public health/readiness routes never install this service.
#[derive(Clone)]
pub struct RuntimeService {
    store: PgJobStore,
    authority: Arc<RuntimeAuthority>,
    capabilities: Arc<Capabilities>,
    artifacts: Arc<ArtifactBroker>,
}

impl RuntimeService {
    /// Compose authenticated RPCs with an already verified, policy-bound store.
    /// No listener, credential generation or production mutation occurs here.
    pub fn new(
        store: PgJobStore,
        authority: Arc<RuntimeAuthority>,
        artifacts: Arc<ArtifactBroker>,
    ) -> Self {
        let store = store.with_runtime_authority(authority.clone());
        Self {
            store,
            authority,
            artifacts,
            capabilities: Arc::new(Capabilities::new()),
        }
    }

    async fn job<T>(
        &self,
        request: &Request<T>,
        id: Option<&str>,
        operation: &str,
        require_capability: bool,
    ) -> StoreResult<(Principal, JobRecord)> {
        let principal = self.authority.authenticate(request)?;
        let id = id.ok_or(StoreError::Invalid("runtime job ID"))?;
        self.authority.authorize_lookup(&principal, id, operation)?;
        let job = self
            .store
            .runtime_job(&principal.actor, id, operation)
            .await
            .map_err(|error| match error {
                StoreError::NotFound => StoreError::AdmissionDenied,
                other => other,
            })?;
        self.authority.authorize(&principal, operation, &job)?;
        if protected(&job) && require_capability {
            self.capabilities.check(
                request,
                &principal,
                &job,
                self.authority.now()?,
                matches!(operation, "loop.jobs.complete" | "loop.jobs.heartbeat"),
            )?;
        } else if request.metadata().contains_key(CAPABILITY_HEADER) {
            return Err(StoreError::AdmissionDenied);
        }
        Ok((principal, job))
    }
}

/// Serve bounded gRPC over explicitly configured mutual TLS. The caller supplies
/// a client CA and server identity; configuration without a client CA is refused
/// by the deployment loader. Cancellation stops accepting connections and allows
/// tonic's bounded in-flight handlers to finish; uncommitted store work rolls back.
/// Returns transport failure without exposing request metadata or secret bytes.
pub async fn serve(
    service: RuntimeService,
    listener: TcpListener,
    tls: ServerTlsConfig,
    shutdown: impl Future<Output = ()>,
) -> StoreResult<()> {
    Server::builder()
        .tls_config(tls.timeout(Duration::from_secs(10)))
        .map_err(|_| StoreError::Invalid("runtime TLS configuration"))?
        .timeout(Duration::from_secs(30))
        .concurrency_limit_per_connection(8)
        .max_concurrent_streams(8)
        .max_connection_age(Duration::from_secs(900))
        .max_connection_age_grace(Duration::from_secs(30))
        .add_service(
            JobServiceServer::new(service)
                .max_decoding_message_size(4 * 1024 * 1024)
                .max_encoding_message_size(4 * 1024 * 1024),
        )
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
        .map_err(|_| StoreError::Unavailable("runtime transport"))
}

#[tonic::async_trait]
impl JobService for RuntimeService {
    async fn get_job(
        &self,
        request: Request<v1::GetJobRequest>,
    ) -> Result<Response<v1::GetJobResponse>, Status> {
        let (_, job) = self
            .job(
                &request,
                request
                    .get_ref()
                    .job_id
                    .as_ref()
                    .map(|id| id.value.as_str()),
                "loop.jobs.read",
                true,
            )
            .await
            .map_err(status)?;
        Ok(Response::new(v1::GetJobResponse { job: Some(job) }))
    }

    async fn acquire_job_lease(
        &self,
        request: Request<v1::AcquireJobLeaseRequest>,
    ) -> Result<Response<v1::AcquireJobLeaseResponse>, Status> {
        let (principal, job) = self
            .job(
                &request,
                request
                    .get_ref()
                    .job_id
                    .as_ref()
                    .map(|id| id.value.as_str()),
                "loop.jobs.acquire",
                false,
            )
            .await
            .map_err(status)?;
        let result = self
            .store
            .mutate(&principal.actor, JobMutation::Acquire(request.into_inner()))
            .await
            .map_err(status)?;
        let mut response = Response::new(v1::AcquireJobLeaseResponse {
            job: Some(result.job),
        });
        if protected(&job) {
            let current = self
                .store
                .runtime_job(
                    &principal.actor,
                    &job.specification
                        .as_ref()
                        .and_then(|spec| spec.job_id.as_ref())
                        .ok_or_else(|| status(StoreError::Corrupt("runtime job")))?
                        .value,
                    "loop.jobs.acquire",
                )
                .await
                .map_err(status)?;
            let receipt_lease = response
                .get_ref()
                .job
                .as_ref()
                .and_then(|job| job.active_lease.as_ref())
                .and_then(|lease| lease.lease_id.as_ref());
            if receipt_lease
                != current
                    .active_lease
                    .as_ref()
                    .and_then(|lease| lease.lease_id.as_ref())
            {
                return Ok(response);
            }
            match self.capabilities.issue(
                &principal,
                &current,
                self.authority.now().map_err(status)?,
            ) {
                Ok(token) => {
                    response.metadata_mut().insert_bin(CAPABILITY_HEADER, token);
                }
                Err(StoreError::LeaseFenced) => (),
                Err(error) => return Err(status(error)),
            }
        }
        Ok(response)
    }

    async fn heartbeat_job_lease(
        &self,
        request: Request<v1::HeartbeatJobLeaseRequest>,
    ) -> Result<Response<v1::HeartbeatJobLeaseResponse>, Status> {
        let (principal, job) = self
            .job(
                &request,
                request
                    .get_ref()
                    .job_id
                    .as_ref()
                    .map(|id| id.value.as_str()),
                "loop.jobs.heartbeat",
                true,
            )
            .await
            .map_err(status)?;
        if protected(&job) {
            self.capabilities
                .command_lease(
                    &request,
                    request
                        .get_ref()
                        .lease_id
                        .as_ref()
                        .map(|id| id.value.as_str()),
                )
                .map_err(status)?;
        }
        let result = self
            .store
            .mutate(
                &principal.actor,
                JobMutation::Heartbeat(request.into_inner()),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(v1::HeartbeatJobLeaseResponse {
            job: Some(result.job),
        }))
    }

    async fn complete_job(
        &self,
        request: Request<v1::CompleteJobRequest>,
    ) -> Result<Response<v1::CompleteJobResponse>, Status> {
        let (principal, job) = self
            .job(
                &request,
                request
                    .get_ref()
                    .job_id
                    .as_ref()
                    .map(|id| id.value.as_str()),
                "loop.jobs.complete",
                true,
            )
            .await
            .map_err(status)?;
        if protected(&job) {
            self.capabilities
                .command_lease(
                    &request,
                    request
                        .get_ref()
                        .lease_id
                        .as_ref()
                        .map(|id| id.value.as_str()),
                )
                .map_err(status)?;
        }
        let result = self
            .store
            .mutate(
                &principal.actor,
                JobMutation::Complete(request.into_inner()),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(v1::CompleteJobResponse {
            job: Some(result.job),
        }))
    }

    async fn cancel_job(
        &self,
        request: Request<v1::CancelJobRequest>,
    ) -> Result<Response<v1::CancelJobResponse>, Status> {
        let (principal, _) = self
            .job(
                &request,
                request
                    .get_ref()
                    .job_id
                    .as_ref()
                    .map(|id| id.value.as_str()),
                "loop.jobs.cancel",
                false,
            )
            .await
            .map_err(status)?;
        let result = self
            .store
            .mutate(&principal.actor, JobMutation::Cancel(request.into_inner()))
            .await
            .map_err(status)?;
        Ok(Response::new(v1::CancelJobResponse {
            job: Some(result.job),
        }))
    }

    async fn prepare_job_artifacts(
        &self,
        request: Request<v1::PrepareJobArtifactsRequest>,
    ) -> Result<Response<v1::PrepareJobArtifactsResponse>, Status> {
        let (principal, job) = self
            .job(
                &request,
                request
                    .get_ref()
                    .job_id
                    .as_ref()
                    .map(|id| id.value.as_str()),
                "loop.jobs.artifacts",
                true,
            )
            .await
            .map_err(status)?;
        let command = request.get_ref();
        if command
            .context
            .as_ref()
            .and_then(|context| context.actor.as_ref())
            != Some(&principal.actor)
        {
            return Err(status(StoreError::AdmissionDenied));
        }
        let lease = &command
            .lease_id
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("data lease")))?
            .value;
        crate::store::live_lease(
            &job,
            &principal.actor,
            lease,
            self.authority.now().map_err(status)?,
        )
        .map_err(status)?;
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            self.artifacts
                .prepare(&self.store, &principal.actor, command, &job),
        )
        .await
        .map_err(|_| status(StoreError::Unavailable("artifact preparation deadline")))?
        .map_err(status)?;
        let (_, current) = self
            .job(
                &request,
                command.job_id.as_ref().map(|id| id.value.as_str()),
                "loop.jobs.artifacts",
                true,
            )
            .await
            .map_err(status)?;
        crate::store::live_lease(
            &current,
            &principal.actor,
            lease,
            self.authority.now().map_err(status)?,
        )
        .map_err(status)?;
        Ok(Response::new(result))
    }
}

pub(super) fn protected(job: &JobRecord) -> bool {
    matches!(
        job.specification
            .as_ref()
            .and_then(|spec| spec.input.as_ref()),
        Some(job_specification::Input::HoldoutBacktest(_))
    )
}

pub(super) fn status(error: StoreError) -> Status {
    use loop_protocol::wire::v1::{ErrorCategory, ServiceError};
    use prost::Message;
    use tonic::Code;
    let (code, category, label, message, retryable) = match error {
        StoreError::AdmissionDenied => (
            Code::PermissionDenied,
            ErrorCategory::Authorization,
            "access_denied",
            "runtime access denied",
            false,
        ),
        StoreError::NotFound => (
            Code::NotFound,
            ErrorCategory::NotFound,
            "not_found",
            "runtime job not found",
            false,
        ),
        StoreError::Invalid(_) | StoreError::Job(_) => (
            Code::InvalidArgument,
            ErrorCategory::Validation,
            "invalid_request",
            "invalid runtime request",
            false,
        ),
        StoreError::RevisionConflict | StoreError::IdempotencyConflict => (
            Code::Aborted,
            ErrorCategory::Conflict,
            "command_conflict",
            "runtime revision or request conflict",
            false,
        ),
        StoreError::LeaseFenced | StoreError::InvalidTransition => (
            Code::FailedPrecondition,
            ErrorCategory::Conflict,
            "lease_fenced",
            "runtime lease or state is not current",
            false,
        ),
        StoreError::Unavailable("artifact preparation deadline" | "data request deadline") => (
            Code::DeadlineExceeded,
            ErrorCategory::Timeout,
            "deadline_exceeded",
            "runtime request deadline",
            true,
        ),
        _ => (
            Code::Unavailable,
            ErrorCategory::Dependency,
            "evidence_unavailable",
            "runtime dependency or evidence unavailable",
            true,
        ),
    };
    let detail = ServiceError {
        category: category as i32,
        code: label.to_owned(),
        message: message.to_owned(),
        retryable,
        details: vec![],
    };
    let rich = RichStatus {
        code: code as i32,
        message: message.to_owned(),
        details: vec![prost_types::Any {
            type_url: loop_protocol::runtime_validation::SERVICE_ERROR_TYPE_URL.to_owned(),
            value: detail.encode_to_vec(),
        }],
    };
    Status::with_details(code, message, rich.encode_to_vec().into())
}

// Wire-compatible google.rpc.Status without an additional runtime dependency.
#[derive(Clone, PartialEq, prost::Message)]
pub(super) struct RichStatus {
    #[prost(int32, tag = "1")]
    pub code: i32,
    #[prost(string, tag = "2")]
    pub message: String,
    #[prost(message, repeated, tag = "3")]
    pub details: Vec<prost_types::Any>,
}
