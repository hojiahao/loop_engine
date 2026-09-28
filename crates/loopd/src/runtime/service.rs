use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

mod reconciliation;
mod statistics;

use loop_protocol::wire::jobs::v1::{
    self,
    job_service_server::{JobService, JobServiceServer},
};
use loop_protocol::wire::v1::{
    JobOutcome, JobRecord, JobState, JobSuccess, job_outcome, job_specification,
};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{
    Request, Response, Status,
    transport::{Server, ServerTlsConfig},
};

use super::authority::{CAPABILITY_HEADER, Principal};
use super::capability::Capabilities;
use super::{
    ArtifactBroker, FactorExecutor, PortfolioExecutor, ReconciliationExecutor, RuntimeAuthority,
    StatisticsExecutor,
};
use crate::store::{
    BacktestRepository, FactorRepository, JobMutation, JobRepository, PgJobStore, StoreError,
    StoreResult,
};

/// Optional mTLS-only job endpoint. The store must use this same deployment
/// authority; public health/readiness routes never install this service.
#[derive(Clone)]
pub struct RuntimeService {
    store: PgJobStore,
    authority: Arc<RuntimeAuthority>,
    capabilities: Arc<Capabilities>,
    artifacts: Arc<ArtifactBroker>,
    evaluator: Option<Arc<FactorExecutor>>,
    portfolio: Option<Arc<PortfolioExecutor>>,
    reconciler: Option<Arc<ReconciliationExecutor>>,
    statistician: Option<Arc<StatisticsExecutor>>,
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
            evaluator: None,
            portfolio: None,
            reconciler: None,
            statistician: None,
        }
    }

    /// Enable only the deployment-pinned numerical implementation. Without this
    /// explicit attachment the evaluation RPC denies, including completed replays.
    pub fn with_factor_executor(mut self, executor: Arc<FactorExecutor>) -> Self {
        self.evaluator = Some(executor);
        self
    }

    /// Attach the pinned installed portfolio producer and its numerical read
    /// policy. Absent deployment configuration denies all portfolio execution.
    pub fn with_portfolio_executor(mut self, executor: Arc<PortfolioExecutor>) -> Self {
        self.store = self.store.with_backtest_policy(executor.clone());
        self.portfolio = Some(executor);
        self
    }

    /// Attach the explicit offline two-engine supervisor. Without it validation
    /// execution, current reads and admission evidence remain unavailable.
    pub fn with_reconciler(mut self, executor: Arc<ReconciliationExecutor>) -> Self {
        self.reconciler = Some(executor);
        self
    }

    /// Enable the installed whole-registry reporter. Without this explicit
    /// deployment attachment execution and current reads remain default-deny.
    pub fn with_statistician(mut self, executor: Arc<StatisticsExecutor>) -> Self {
        self.statistician = Some(executor);
        self
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
        .timeout(Duration::from_secs(240))
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
    async fn execute_statistics(
        &self,
        request: Request<v1::ExecuteStatisticsRequest>,
    ) -> Result<Response<v1::ExecuteStatisticsResponse>, Status> {
        self.execute_global(request)
            .await
            .map(Response::new)
            .map_err(status)
    }

    async fn read_statistics(
        &self,
        request: Request<v1::ReadStatisticsRequest>,
    ) -> Result<Response<v1::ReadStatisticsResponse>, Status> {
        self.read_global(request)
            .await
            .map(Response::new)
            .map_err(status)
    }
    async fn read_backtest(
        &self,
        request: Request<v1::ReadBacktestRequest>,
    ) -> Result<Response<v1::ReadBacktestResponse>, Status> {
        let command = request.get_ref();
        let (principal, _) = self
            .job(
                &request,
                command.job_id.as_ref().map(|id| id.value.as_str()),
                "loop.backtests.read_current",
                false,
            )
            .await
            .map_err(status)?;
        let result = self
            .store
            .current_backtest(
                &principal.actor,
                &command
                    .job_id
                    .as_ref()
                    .ok_or_else(|| status(StoreError::Invalid("backtest ID")))?
                    .value,
                &command.context_id,
            )
            .await
            .map_err(status)?;
        Ok(Response::new(v1::ReadBacktestResponse {
            result: Some(result),
        }))
    }

    async fn export_backtest(
        &self,
        request: Request<v1::ExportBacktestRequest>,
    ) -> Result<Response<v1::ExportBacktestResponse>, Status> {
        let command = request.get_ref();
        let (principal, _) = self
            .job(
                &request,
                command.job_id.as_ref().map(|id| id.value.as_str()),
                "loop.backtests.export_current",
                false,
            )
            .await
            .map_err(status)?;
        let result = self
            .store
            .export_current(
                &principal.actor,
                crate::store::ExportBacktest {
                    context: command.context.clone(),
                    job_id: command.job_id.clone(),
                    context_id: command.context_id.clone(),
                    deadline: command.deadline,
                },
            )
            .await
            .map_err(status)?;
        Ok(Response::new(v1::ExportBacktestResponse {
            result: Some(result.result),
            accepted_at: Some(result.accepted_at),
            replayed: result.replayed,
        }))
    }

    async fn decide_factor(
        &self,
        request: Request<v1::DecideFactorRequest>,
    ) -> Result<Response<v1::DecideFactorResponse>, Status> {
        let command = request.get_ref();
        let (principal, _) = self
            .job(
                &request,
                command.source_job_id.as_ref().map(|id| id.value.as_str()),
                "loop.factors.decide",
                false,
            )
            .await
            .map_err(status)?;
        crate::store::validate_runtime_context(command.context.as_ref(), &principal.actor)
            .map_err(status)?;
        let decision_store = self
            .decision_store(&request, &principal)
            .await
            .map_err(status)?;
        let result = decision_store
            .decide_factor(
                &principal.actor,
                crate::store::DecideFactor {
                    context: command.context.clone(),
                    source_job_id: command.source_job_id.clone(),
                    context_id: command.context_id.clone(),
                    expected_revision: command.expected_revision,
                    reason: command.reason.clone(),
                    override_reason: command.override_reason.clone(),
                    override_approval_id: command.override_approval_id.clone(),
                    deadline: command.deadline,
                },
            )
            .await
            .map_err(status)?;
        Ok(Response::new(v1::DecideFactorResponse {
            states: result
                .states
                .into_iter()
                .map(|state| v1::FactorDecisionState {
                    factor_spec_id: Some(loop_protocol::wire::v1::FactorSpecId {
                        value: state.factor_spec_id,
                    }),
                    revision: state.revision,
                    status: state.status,
                    admissions: state.admissions,
                    retirements: state.retirements,
                    source_job_id: Some(loop_protocol::wire::v1::JobId {
                        value: state.source_job_id,
                    }),
                })
                .collect(),
            rejection_code: result.rejection_code,
            override_applied: result.override_applied,
            accepted_at: Some(result.accepted_at),
            replayed: result.replayed,
        }))
    }

    async fn execute_reconciliation(
        &self,
        request: Request<v1::ExecuteReconciliationRequest>,
    ) -> Result<Response<v1::ExecuteReconciliationResponse>, Status> {
        self.execute_validation(request)
            .await
            .map(Response::new)
            .map_err(status)
    }

    async fn read_reconciliation(
        &self,
        request: Request<v1::ReadReconciliationRequest>,
    ) -> Result<Response<v1::ReadReconciliationResponse>, Status> {
        self.read_validation(request)
            .await
            .map(Response::new)
            .map_err(status)
    }

    async fn execute_backtest(
        &self,
        request: Request<v1::ExecuteBacktestRequest>,
    ) -> Result<Response<v1::ExecuteBacktestResponse>, Status> {
        let command = request.get_ref();
        let id = command.job_id.as_ref().map(|id| id.value.as_str());
        let (principal, job) = self
            .job(&request, id, "loop.jobs.backtest", false)
            .await
            .map_err(status)?;
        crate::store::validate_runtime_context(command.context.as_ref(), &principal.actor)
            .map_err(status)?;
        let executor = self
            .portfolio
            .as_ref()
            .ok_or_else(|| status(StoreError::AdmissionDenied))?;
        let specification = job
            .specification
            .as_ref()
            .ok_or_else(|| status(StoreError::Corrupt("portfolio job")))?;
        let lease = command
            .lease_id
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("portfolio lease")))?;
        // Fence cancelled/expired work before any expensive input reads.
        if job.state != JobState::Succeeded as i32 {
            if job.revision != command.expected_revision {
                return Err(status(StoreError::RevisionConflict));
            }
            crate::store::live_lease(
                &job,
                &principal.actor,
                &lease.value,
                self.authority.now().map_err(status)?,
            )
            .map_err(status)?;
        }
        let inputs = executor.inputs(specification).await.map_err(status)?;
        let prepared = if job.state == JobState::Succeeded as i32 {
            let Some(job_outcome::Outcome::Success(success)) = job
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            else {
                return Err(status(StoreError::Corrupt("portfolio replay outcome")));
            };
            executor
                .replay(inputs, specification, success)
                .await
                .map_err(status)?
        } else {
            let trials = self
                .store
                .trial_ledger(&principal.actor)
                .await
                .map_err(status)?;
            self.store
                .portfolio_source(&principal.actor, &job, &inputs.lineage(trials.clone()))
                .await
                .map_err(status)?;
            let prepared = self
                .artifacts
                .prepare(
                    &self.store,
                    &principal.actor,
                    &v1::PrepareJobArtifactsRequest {
                        context: command.context.clone(),
                        job_id: command.job_id.clone(),
                        lease_id: command.lease_id.clone(),
                        expected_revision: command.expected_revision,
                    },
                    &job,
                )
                .await
                .map_err(status)?;
            let view = self
                .artifacts
                .evaluation_view(&prepared)
                .await
                .map_err(status)?;
            let (_, current) = self
                .job(&request, id, "loop.jobs.backtest", false)
                .await
                .map_err(status)?;
            if current.revision != command.expected_revision {
                return Err(status(StoreError::RevisionConflict));
            }
            let now = self.authority.now().map_err(status)?;
            let expires = crate::store::live_lease(&current, &principal.actor, &lease.value, now)
                .map_err(status)?;
            let remaining = Duration::from_millis(
                u64::try_from(expires - now).map_err(|_| status(StoreError::LeaseFenced))?,
            )
            .min(Duration::from_secs(180));
            let work = inputs.work(&lease.value, trials, None);
            executor
                .execute(inputs, &work, &view, remaining)
                .await
                .map_err(status)?
        };
        if prepared.lease_id.as_deref() != Some(lease.value.as_str()) {
            return Err(status(StoreError::LeaseFenced));
        }
        let success = prepared
            .success
            .clone()
            .ok_or_else(|| status(StoreError::Corrupt("portfolio output set")))?;
        self.job(&request, id, "loop.jobs.backtest", false)
            .await
            .map_err(status)?;
        let completed = self
            .store
            .with_backtest_policy(Arc::new(prepared))
            .mutate(
                &principal.actor,
                JobMutation::Complete(v1::CompleteJobRequest {
                    context: command.context.clone(),
                    job_id: command.job_id.clone(),
                    lease_id: command.lease_id.clone(),
                    expected_revision: command.expected_revision,
                    outcome: Some(JobOutcome {
                        outcome: Some(job_outcome::Outcome::Success(success)),
                    }),
                }),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(v1::ExecuteBacktestResponse {
            job: Some(completed.job),
        }))
    }

    async fn evaluate_factor(
        &self,
        request: Request<v1::EvaluateFactorRequest>,
    ) -> Result<Response<v1::EvaluateFactorResponse>, Status> {
        let (principal, job) = self
            .job(
                &request,
                request
                    .get_ref()
                    .job_id
                    .as_ref()
                    .map(|id| id.value.as_str()),
                "loop.jobs.evaluate",
                false,
            )
            .await
            .map_err(status)?;
        let executor = self
            .evaluator
            .as_ref()
            .ok_or_else(|| status(StoreError::AdmissionDenied))?;
        let _permit = executor
            .permits
            .try_acquire()
            .map_err(|_| status(StoreError::Unavailable("factor execution capacity")))?;
        let command = request.get_ref();
        crate::store::validate_runtime_context(command.context.as_ref(), &principal.actor)
            .map_err(status)?;
        let specification = job
            .specification
            .as_ref()
            .ok_or_else(|| status(StoreError::Corrupt("evaluation job")))?;
        let lease = command
            .lease_id
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("evaluation lease")))?;
        if job.state != JobState::Succeeded as i32 {
            if job.revision != command.expected_revision {
                return Err(status(StoreError::RevisionConflict));
            }
            crate::store::live_lease(
                &job,
                &principal.actor,
                &lease.value,
                self.authority.now().map_err(status)?,
            )
            .map_err(status)?;
        }
        let inputs = executor
            .resolver
            .prepare(specification, lease)
            .await
            .map_err(status)?;
        let result;
        let success = if job.state == JobState::Succeeded as i32 {
            executor
                .run(&inputs.work, None, Duration::from_secs(30))
                .await
                .map_err(status)?;
            result = None;
            match job
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            {
                Some(job_outcome::Outcome::Success(success)) => success.clone(),
                _ => return Err(status(StoreError::Corrupt("evaluation replay outcome"))),
            }
        } else {
            let preparation = v1::PrepareJobArtifactsRequest {
                context: command.context.clone(),
                job_id: command.job_id.clone(),
                lease_id: command.lease_id.clone(),
                expected_revision: command.expected_revision,
            };
            let prepared = self
                .artifacts
                .prepare(&self.store, &principal.actor, &preparation, &job)
                .await
                .map_err(status)?;
            let view = self
                .artifacts
                .evaluation_view(&prepared)
                .await
                .map_err(status)?;
            // Preparation consumes the original lease and job budget. Recheck
            // current state immediately before spawn; never grant a fresh
            // wall-clock budget merely because artifact verification finished.
            let (_, current) = self
                .job(
                    &request,
                    command.job_id.as_ref().map(|id| id.value.as_str()),
                    "loop.jobs.evaluate",
                    false,
                )
                .await
                .map_err(status)?;
            if current.revision != command.expected_revision {
                return Err(status(StoreError::RevisionConflict));
            }
            let now = self.authority.now().map_err(status)?;
            let expires = crate::store::live_lease(&current, &principal.actor, &lease.value, now)
                .map_err(status)?;
            let remaining = Duration::from_millis(
                u64::try_from(expires - now).map_err(|_| status(StoreError::LeaseFenced))?,
            )
            .min(Duration::from_secs(60));
            let response = executor
                .run(&inputs.work, Some(&view), remaining)
                .await
                .map_err(status)?
                .ok_or_else(|| status(StoreError::Corrupt("missing evaluation result")))?;
            let success = JobSuccess {
                outputs: vec![
                    response
                        .values
                        .clone()
                        .ok_or_else(|| status(StoreError::Corrupt("missing factor values")))?,
                    response
                        .manifest
                        .clone()
                        .ok_or_else(|| status(StoreError::Corrupt("missing factor manifest")))?,
                ],
            };
            result = Some(response);
            success
        };
        let evidence = inputs
            .resolve(&executor.outputs, &success)
            .await
            .map_err(status)?;
        if result
            .as_ref()
            .is_some_and(|result| result != &evidence.result)
        {
            return Err(status(StoreError::Corrupt(
                "worker/manifest result mismatch",
            )));
        }
        self.job(
            &request,
            command.job_id.as_ref().map(|id| id.value.as_str()),
            "loop.jobs.evaluate",
            false,
        )
        .await
        .map_err(status)?;
        executor.check_output().map_err(status)?;
        let completed = self
            .store
            .with_evaluation_evidence(evidence)
            .mutate(
                &principal.actor,
                JobMutation::Complete(v1::CompleteJobRequest {
                    context: command.context.clone(),
                    job_id: command.job_id.clone(),
                    lease_id: command.lease_id.clone(),
                    expected_revision: command.expected_revision,
                    outcome: Some(JobOutcome {
                        outcome: Some(job_outcome::Outcome::Success(success)),
                    }),
                }),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(v1::EvaluateFactorResponse {
            job: Some(completed.job),
        }))
    }

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
        if matches!(
            job.specification
                .as_ref()
                .and_then(|specification| specification.input.as_ref()),
            Some(job_specification::Input::Backtest(_))
        ) && matches!(
            request
                .get_ref()
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref()),
            Some(job_outcome::Outcome::Success(_) | job_outcome::Outcome::FactorRejection(_))
        ) {
            return Err(status(StoreError::AdmissionDenied));
        }
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
    // Only static, service-owned reason labels reach operator logs. Database
    // errors, paths and caller-supplied metadata are deliberately not formatted.
    if let StoreError::Unavailable(reason) | StoreError::Corrupt(reason) = &error {
        tracing::warn!(reason, "runtime operation failed closed");
    }
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
        StoreError::StaleTrials => (
            Code::Aborted,
            ErrorCategory::Conflict,
            "trial_history_changed",
            "global research trial accounting changed",
            false,
        ),
        StoreError::PreviouslyRejected => (
            Code::FailedPrecondition,
            ErrorCategory::Conflict,
            "previously_rejected",
            "frozen research context already rejected",
            false,
        ),
        StoreError::AlreadyEvaluated => (
            Code::AlreadyExists,
            ErrorCategory::Conflict,
            "already_evaluated",
            "frozen factor context already evaluated",
            false,
        ),
        StoreError::IndependentPending => (
            Code::FailedPrecondition,
            ErrorCategory::Dependency,
            "independent_validation_pending",
            "independent portfolio reconciliation pending",
            false,
        ),
        StoreError::IndependentMismatch => (
            Code::FailedPrecondition,
            ErrorCategory::Dependency,
            "independent_validation_differs",
            "independent portfolio reconciliation differs",
            false,
        ),
        StoreError::IndependentUnavailable => (
            Code::FailedPrecondition,
            ErrorCategory::Dependency,
            "independent_validation_unavailable",
            "independent portfolio reconciliation unavailable",
            false,
        ),
        StoreError::AdmissionPrerequisite => (
            Code::FailedPrecondition,
            ErrorCategory::Dependency,
            "production_admission_prerequisites",
            "production admission requires licensed data, frozen statistical acceptance and semantic review",
            false,
        ),
        StoreError::StatisticsPending => (
            Code::FailedPrecondition,
            ErrorCategory::Dependency,
            "global_statistics_pending",
            "global statistical evidence pending",
            false,
        ),
        StoreError::StatisticsUnavailable => (
            Code::FailedPrecondition,
            ErrorCategory::Dependency,
            "global_statistics_unavailable",
            "global statistical evidence unavailable",
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
