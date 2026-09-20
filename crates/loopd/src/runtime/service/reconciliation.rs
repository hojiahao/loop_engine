//! Narrow authenticated handlers; numerical reports never authorize themselves.

use super::*;
use crate::manifests::reconciliation::{ValidationEvidence, source};
use crate::runtime::ValidationTask;

impl RuntimeService {
    pub(super) async fn execute_validation(
        &self,
        request: Request<v1::ExecuteReconciliationRequest>,
    ) -> StoreResult<v1::ExecuteReconciliationResponse> {
        let command = request.get_ref();
        let id = command.job_id.as_ref().map(|id| id.value.as_str());
        let (principal, job) = self.job(&request, id, "loop.jobs.reconcile", false).await?;
        crate::store::validate_runtime_context(command.context.as_ref(), &principal.actor)?;
        let lease = &command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("validation lease"))?
            .value;
        let remaining = if job.state == JobState::Succeeded as i32 {
            Duration::from_secs(180)
        } else {
            if job.revision != command.expected_revision {
                return Err(StoreError::RevisionConflict);
            }
            let now = self.authority.now()?;
            let expires = crate::store::live_lease(&job, &principal.actor, lease, now)?;
            Duration::from_millis(
                u64::try_from(expires - now).map_err(|_| StoreError::LeaseFenced)?,
            )
            .min(Duration::from_secs(180))
        };
        let evidence = tokio::time::timeout(
            remaining,
            self.validation_evidence(&principal, &job, Some(lease)),
        )
        .await
        .map_err(|_| StoreError::Unavailable("reconciliation deadline"))??;
        self.job(&request, id, "loop.jobs.reconcile", false).await?;
        let success = evidence.success()?;
        let completed = self
            .store
            .with_validation_evidence(evidence)
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
            .await?;
        Ok(v1::ExecuteReconciliationResponse {
            job: Some(completed.job),
        })
    }

    pub(super) async fn read_validation(
        &self,
        request: Request<v1::ReadReconciliationRequest>,
    ) -> StoreResult<v1::ReadReconciliationResponse> {
        let id = request
            .get_ref()
            .job_id
            .as_ref()
            .map(|id| id.value.as_str());
        let (principal, job) = self
            .job(&request, id, "loop.reconciliation.read_current", false)
            .await?;
        if job.state != JobState::Succeeded as i32 {
            return Err(StoreError::InvalidTransition);
        }
        let evidence = tokio::time::timeout(
            Duration::from_secs(180),
            self.validation_evidence(&principal, &job, None),
        )
        .await
        .map_err(|_| StoreError::Unavailable("reconciliation read deadline"))??;
        self.job(&request, id, "loop.reconciliation.read_current", false)
            .await?;
        let store = self.store.with_validation_evidence(evidence.clone());
        let current = store
            .current_validation(&principal.actor, &evidence)
            .await?;
        Ok(v1::ReadReconciliationResponse {
            job: Some(current),
            report: Some(evidence.report.wire()?),
        })
    }

    async fn validation_evidence(
        &self,
        principal: &Principal,
        record: &JobRecord,
        supplied_lease: Option<&str>,
    ) -> StoreResult<Arc<ValidationEvidence>> {
        let executor = self
            .reconciler
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let portfolio = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("validation job"))?;
        let (source_id, context) = source(job)?;
        let primary_record = self
            .store
            .runtime_job(&principal.actor, source_id, "loop.backtests.read_current")
            .await?;
        self.authority
            .authorize(principal, "loop.backtests.read_current", &primary_record)?;
        if primary_record.state != JobState::Succeeded as i32 {
            return Err(StoreError::InvalidTransition);
        }
        let source_job = primary_record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("primary source"))?;
        // Refuse other job kinds before opening any data namespace or worker.
        if !matches!(
            source_job.input,
            Some(job_specification::Input::Backtest(_))
        ) {
            return Err(StoreError::AdmissionDenied);
        }
        let Some(job_outcome::Outcome::Success(success)) = primary_record
            .outcome
            .as_ref()
            .and_then(|value| value.outcome.as_ref())
        else {
            return Err(StoreError::Corrupt("primary outcome"));
        };
        let inputs = portfolio.inputs(source_job).await?;
        let prepared = Arc::new(portfolio.replay(inputs, source_job, success).await?);
        self.store
            .with_backtest_policy(prepared.clone())
            .current_backtest(&principal.actor, source_id, &context)
            .await?;
        let prior = if record.state == JobState::Succeeded as i32 {
            match record
                .outcome
                .as_ref()
                .and_then(|value| value.outcome.as_ref())
            {
                Some(job_outcome::Outcome::Success(success)) => Some(success),
                _ => return Err(StoreError::Corrupt("validation historical outcome")),
            }
        } else {
            None
        };
        let original_lease;
        let lease = match supplied_lease {
            Some(lease) => lease,
            None => {
                original_lease = executor
                    .lease(prior.ok_or(StoreError::InvalidTransition)?)
                    .await?;
                &original_lease
            }
        };
        executor
            .execute(
                ValidationTask {
                    job,
                    lease,
                    record: primary_record,
                    primary: prepared,
                    prior,
                    started_ms: crate::store::timestamp_millis(
                        record
                            .active_lease
                            .as_ref()
                            .and_then(|lease| lease.issued_at.as_ref())
                            .or(record.updated_at.as_ref())
                            .ok_or(StoreError::Corrupt("validation attempt time"))?,
                        false,
                    )?,
                },
                Duration::from_secs(180),
            )
            .await
            .map(Arc::new)
    }

    pub(super) async fn decision_store(
        &self,
        request: &Request<v1::DecideFactorRequest>,
        principal: &Principal,
    ) -> StoreResult<PgJobStore> {
        let Some(executor) = &self.reconciler else {
            return Ok(self.store.clone());
        };
        let id = &request
            .get_ref()
            .source_job_id
            .as_ref()
            .ok_or(StoreError::Invalid("decision source"))?
            .value;
        let Some(validation_id) = executor.for_primary(id) else {
            return Ok(self.store.clone());
        };
        let command = request.get_ref();
        let requested = crate::store::timestamp_millis(
            command
                .context
                .as_ref()
                .and_then(|context| context.requested_at.as_ref())
                .ok_or(StoreError::Invalid("decision request time"))?,
            true,
        )?;
        let deadline = crate::store::timestamp_millis(
            command
                .deadline
                .as_ref()
                .ok_or(StoreError::Invalid("factor deadline"))?,
            true,
        )?;
        let now = self.authority.now()?;
        if deadline <= requested
            || deadline - requested > crate::store::VALIDATION_DEADLINE_MS
            || now < requested
            || now >= deadline
        {
            return Err(StoreError::Invalid("factor deadline"));
        }
        let (_, job) = self
            .job(
                request,
                Some(validation_id),
                "loop.reconciliation.read_current",
                false,
            )
            .await?;
        if job.state != JobState::Succeeded as i32 {
            return Err(StoreError::IndependentPending);
        }
        let current = self.authority.now()?;
        if current < now || current >= deadline {
            return Err(StoreError::Invalid("factor deadline"));
        }
        let remaining = Duration::from_millis(
            u64::try_from(deadline - current)
                .map_err(|_| StoreError::Invalid("factor deadline"))?,
        );
        let evidence =
            tokio::time::timeout(remaining, self.validation_evidence(principal, &job, None))
                .await
                .map_err(|_| StoreError::Unavailable("admission validation deadline"))??;
        Ok(self
            .store
            .with_backtest_policy(evidence.primary.clone())
            .with_validation_evidence(evidence))
    }
}
