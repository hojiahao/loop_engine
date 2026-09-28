//! No RPC-supplied population, metrics, paths or permissions are accepted.

use super::*;
use crate::manifests::statistics::{StatisticsEvidence, is_statistics};
use crate::runtime::StatisticsTask;

impl RuntimeService {
    pub(super) async fn execute_global(
        &self,
        request: Request<v1::ExecuteStatisticsRequest>,
    ) -> StoreResult<v1::ExecuteStatisticsResponse> {
        let command = request.get_ref();
        let id = command.job_id.as_ref().map(|id| id.value.as_str());
        let (principal, record) = self
            .job(&request, id, "loop.jobs.statistics", false)
            .await?;
        crate::store::validate_runtime_context(command.context.as_ref(), &principal.actor)?;
        let lease = &command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("statistics lease"))?
            .value;
        let remaining = if record.state == JobState::Succeeded as i32 {
            Duration::from_secs(180)
        } else {
            if record.revision != command.expected_revision {
                return Err(StoreError::RevisionConflict);
            }
            let now = self.authority.now()?;
            let expires = crate::store::live_lease(&record, &principal.actor, lease, now)?;
            Duration::from_millis(
                u64::try_from(expires - now).map_err(|_| StoreError::LeaseFenced)?,
            )
            .min(Duration::from_secs(180))
        };
        let evidence = tokio::time::timeout(
            remaining,
            self.statistical_evidence(&principal, &record, Some(lease)),
        )
        .await
        .map_err(|_| StoreError::Unavailable("statistics deadline"))??;
        self.job(&request, id, "loop.jobs.statistics", false)
            .await?;
        let success = evidence.success()?;
        let completed = self
            .store
            .with_statistics_evidence(evidence)
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
        Ok(v1::ExecuteStatisticsResponse {
            job: Some(completed.job),
        })
    }

    pub(super) async fn read_global(
        &self,
        request: Request<v1::ReadStatisticsRequest>,
    ) -> StoreResult<v1::ReadStatisticsResponse> {
        let id = request
            .get_ref()
            .job_id
            .as_ref()
            .map(|id| id.value.as_str());
        let (principal, record) = self
            .job(&request, id, "loop.statistics.read_current", false)
            .await?;
        if record.state != JobState::Succeeded as i32 {
            return Err(StoreError::InvalidTransition);
        }
        let proof = tokio::time::timeout(
            Duration::from_secs(180),
            self.statistical_evidence(&principal, &record, None),
        )
        .await
        .map_err(|_| StoreError::Unavailable("statistics read deadline"))??;
        self.job(&request, id, "loop.statistics.read_current", false)
            .await?;
        let current = self
            .store
            .with_statistics_evidence(proof.clone())
            .current_statistics(&principal.actor, &proof)
            .await?;
        Ok(v1::ReadStatisticsResponse {
            job: Some(current),
            report: Some(proof.report.wire()?),
        })
    }

    pub(super) async fn statistical_evidence(
        &self,
        principal: &Principal,
        record: &JobRecord,
        supplied_lease: Option<&str>,
    ) -> StoreResult<Arc<StatisticsEvidence>> {
        let executor = self
            .statistician
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let portfolio = self.portfolio.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("statistics job"))?;
        if !is_statistics(job) {
            return Err(StoreError::AdmissionDenied);
        }
        let snapshot = self.store.trial_snapshot(&principal.actor).await?;
        let mut portfolios = Vec::new();
        for source in &snapshot.records {
            self.authority
                .authorize(principal, "loop.factors.read_trials", source)?;
            if source.state != JobState::Succeeded as i32 {
                continue;
            }
            let source_job = source
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("statistics source"))?;
            if !matches!(
                source_job.input,
                Some(job_specification::Input::Backtest(_))
            ) {
                continue;
            }
            if portfolios.len() == 64 {
                return Err(StoreError::Invalid("global portfolio bounds"));
            }
            self.authority
                .authorize(principal, "loop.backtests.read_current", source)?;
            let Some(job_outcome::Outcome::Success(success)) = source
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            else {
                return Err(StoreError::Corrupt("statistics source outcome"));
            };
            let inputs = portfolio.inputs(source_job).await?;
            // Authenticate the registered bytes and lineage here. The global
            // worker reconstructs every source numerically once; replaying it
            // here as well would double work without adding independent proof.
            let artifact = crate::runtime::portfolio::manifest_artifact(success)?;
            let prepared = Arc::new(inputs.seal(&portfolio.outputs, &artifact).await?);
            if prepared.success.as_ref() != Some(success) {
                return Err(StoreError::Corrupt("statistics source outputs"));
            }
            self.store
                .with_backtest_policy(prepared.clone())
                .statistics_source(&principal.actor, source, &prepared, &snapshot.ledger)
                .await?;
            portfolios.push((source.clone(), prepared));
        }
        let prior = if record.state == JobState::Succeeded as i32 {
            match record
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            {
                Some(job_outcome::Outcome::Success(success)) => Some(success),
                _ => return Err(StoreError::Corrupt("statistics historical outcome")),
            }
        } else {
            None
        };
        let original;
        let lease = match supplied_lease {
            Some(lease) => lease,
            None => {
                original = executor
                    .identity(prior.ok_or(StoreError::InvalidTransition)?)
                    .await?;
                &original.lease_id
            }
        };
        executor
            .execute(StatisticsTask {
                job,
                lease,
                snapshot,
                portfolios,
                prior,
                started_ms: crate::store::timestamp_millis(
                    record
                        .active_lease
                        .as_ref()
                        .and_then(|lease| lease.issued_at.as_ref())
                        .or(record.updated_at.as_ref())
                        .ok_or(StoreError::Corrupt("statistics attempt time"))?,
                    false,
                )?,
            })
            .await
            .map(Arc::new)
    }
}
