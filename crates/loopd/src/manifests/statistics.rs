//! Whole-registry report models and operation-scoped, non-importable evidence.

use std::sync::Arc;

use loop_protocol::wire::v1::{
    JobKind, JobRecord, JobSpecification, JobSuccess, job_outcome, job_specification,
};
use serde::{Deserialize, Serialize};

use super::files::VerifiedFile;
use super::portfolio::PreparedPortfolio;
use super::{ObjectRef, model};
use crate::store::{StoreError, StoreResult, TrialLedger, TrialSnapshot};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatisticsPolicy {
    pub schema: String,
    pub policy_id: String,
    pub revision: String,
    pub scope: String,
    pub minimum_sessions: u32,
    pub hac_lags: u32,
    pub pbo_blocks: u32,
}

impl StatisticsPolicy {
    pub(crate) fn validate(&self) -> StoreResult<()> {
        crate::store::validate_id(&self.policy_id)?;
        crate::store::validate_id(&self.revision)?;
        if self.schema != "loop.global-statistics-policy/v1"
            || self.revision.starts_with('0')
            || !self.revision.bytes().all(|value| value.is_ascii_digit())
            || self.revision.parse::<u64>().is_err()
            || self.scope != "all-database-development-trials"
            || !(8..=8192).contains(&self.minimum_sessions)
            || self.hac_lags > 60
            || self.hac_lags >= self.minimum_sessions
            || !(4..=10).contains(&self.pbo_blocks)
            || !self.pbo_blocks.is_multiple_of(2)
        {
            return Err(StoreError::Invalid("global statistical policy"));
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub(crate) struct TrialState {
    job_id: String,
    revision: u64,
    kind: &'static str,
    state: i32,
    attempt: u32,
}

#[derive(Serialize)]
pub(crate) struct GlobalSnapshot {
    schema: &'static str,
    ledger: TrialLedger,
    states: Vec<TrialState>,
}

impl GlobalSnapshot {
    pub(crate) fn project(snapshot: &TrialSnapshot) -> StoreResult<Self> {
        let mut states = Vec::with_capacity(snapshot.records.len());
        for record in &snapshot.records {
            let job = record
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("trial job"))?;
            let kind = match JobKind::try_from(job.kind) {
                Ok(JobKind::FactorEvaluation) => "factor_evaluation",
                Ok(JobKind::Backtest) => "backtest",
                _ => return Err(StoreError::Corrupt("global trial kind")),
            };
            states.push(TrialState {
                job_id: job
                    .job_id
                    .as_ref()
                    .ok_or(StoreError::Corrupt("trial ID"))?
                    .value
                    .clone(),
                revision: record.revision,
                kind,
                state: record.state,
                attempt: record.attempt,
            });
        }
        Ok(Self {
            schema: "loop.global-snapshot/v1",
            ledger: snapshot.ledger.clone(),
            states,
        })
    }
}

#[derive(Serialize)]
pub(crate) struct GlobalPortfolio {
    pub job_id: String,
    pub evaluation_job_id: String,
    pub lease_id: String,
    pub specification: ObjectRef,
    pub request: ObjectRef,
    pub manifest: ObjectRef,
}

#[derive(Serialize)]
pub(crate) struct StatisticsWork {
    pub schema: &'static str,
    pub job_id: String,
    pub lease_id: String,
    pub started_at_ms: i64,
    pub policy: ObjectRef,
    pub snapshot: GlobalSnapshot,
    pub portfolios: Vec<GlobalPortfolio>,
    pub manifest: Option<ObjectRef>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatisticsDocument {
    pub schema: String,
    pub job_id: String,
    pub lease_id: String,
    pub started_at_ms: i64,
    pub policy: ObjectRef,
    pub snapshot: ObjectRef,
    pub request: ObjectRef,
    pub summary: ObjectRef,
    pub matrix: ObjectRef,
    pub production_eligible: bool,
}

/// Only an actual supervised worker may construct this proof. Neither RPC
/// output artifacts nor caller metadata can substitute for these live guards.
pub(crate) struct StatisticsEvidence {
    pub job: JobSpecification,
    pub snapshot: TrialSnapshot,
    pub portfolios: Vec<(JobRecord, Arc<PreparedPortfolio>)>,
    pub document: StatisticsDocument,
    pub report: model::Artifact,
    pub files: Vec<Arc<VerifiedFile>>,
}

impl StatisticsEvidence {
    pub(crate) fn check(&self) -> StoreResult<()> {
        for (record, portfolio) in &self.portfolios {
            portfolio.check(
                record
                    .specification
                    .as_ref()
                    .ok_or(StoreError::Corrupt("statistical source"))?,
            )?;
        }
        for file in &self.files {
            file.check()?;
        }
        Ok(())
    }

    pub(crate) fn success(&self) -> StoreResult<JobSuccess> {
        self.check()?;
        Ok(JobSuccess {
            outputs: vec![self.report.wire()?],
        })
    }

    pub(crate) fn check_record(&self, record: &JobRecord) -> StoreResult<()> {
        if record.specification.as_ref() != Some(&self.job) {
            return Err(StoreError::Corrupt("statistics job binding"));
        }
        let Some(job_outcome::Outcome::Success(success)) = record
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref())
        else {
            return Err(StoreError::Corrupt("statistics outcome"));
        };
        if *success != self.success()? {
            return Err(StoreError::Corrupt("statistics output binding"));
        }
        Ok(())
    }
}

pub(crate) fn is_statistics(job: &JobSpecification) -> bool {
    job.kind == JobKind::Report as i32
        && matches!(&job.input, Some(job_specification::Input::Artifact(input))
            if input.input.as_ref().and_then(|input| input.schema.as_ref())
                .is_some_and(|schema| schema.name == "loop.global_statistics_policy" && schema.version == 1))
}
