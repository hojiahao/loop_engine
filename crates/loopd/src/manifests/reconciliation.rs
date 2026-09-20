//! Bound independent receipts. Only the installed runtime constructs authority.

use std::sync::Arc;

use loop_protocol::wire::v1::{
    JobRecord, JobSpecification, JobSuccess, job_outcome, job_specification,
};
use serde::{Deserialize, Serialize};

use super::files::VerifiedFile;
use super::portfolio::{PortfolioWork, PreparedPortfolio};
use super::{ObjectRef, model};
use crate::store::{StoreError, StoreResult};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComparisonPolicy {
    pub schema: String,
    pub policy_id: String,
    pub revision: String,
    pub profile: String,
    pub statistics_absolute: String,
    pub statistics_relative: String,
    pub price_absolute: String,
    pub dollar_absolute: String,
    pub return_absolute: String,
    pub accounting_relative: String,
}

impl ComparisonPolicy {
    pub(crate) fn validate(&self) -> StoreResult<()> {
        model::schema(&self.schema, "loop.reconciliation-policy/v1")?;
        if self.profile != "alphalens-zipline-development.1"
            || self.statistics_absolute != "0.000000000001"
            || self.statistics_relative != "0.0000000001"
            || self.price_absolute != "0.000000005"
            || self.dollar_absolute != "0.00001"
            || self.return_absolute != "0.000000000001"
            || self.accounting_relative != "0"
        {
            return Err(StoreError::Invalid("unsupported frozen comparison policy"));
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub(crate) struct ValidationWork {
    pub schema: &'static str,
    pub job_id: String,
    pub lease_id: String,
    pub primary_revision: u64,
    pub primary: PortfolioWork,
    pub policy: ObjectRef,
    pub prepared: Option<ObjectRef>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedInputs {
    pub schema: String,
    pub job_id: String,
    pub lease_id: String,
    pub primary_job_id: String,
    pub primary_revision: u64,
    pub primary_manifest: ObjectRef,
    pub policy: ObjectRef,
    pub statistics: ObjectRef,
    pub alphalens: ObjectRef,
    pub zipline: ObjectRef,
    pub production_eligible: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedHandle {
    pub reference: ObjectRef,
    pub inputs: PreparedInputs,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Disposition {
    Accepted,
    Rejected,
    Unavailable,
}

impl Disposition {
    pub(crate) fn combined(self, other: Self) -> Self {
        match (self, other) {
            (Self::Rejected, _) | (_, Self::Rejected) => Self::Rejected,
            (Self::Unavailable, _) | (_, Self::Unavailable) => Self::Unavailable,
            _ => Self::Accepted,
        }
    }

    pub(crate) fn exit_code(self) -> i32 {
        match self {
            Self::Accepted => 0,
            Self::Rejected => 3,
            Self::Unavailable => 4,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AlphalensReceipt {
    pub schema: String,
    pub inputs: ObjectRef,
    pub build: ObjectRef,
    pub cross_sections: ObjectRef,
    pub turnover: ObjectRef,
    pub differences: ObjectRef,
    pub summary: ObjectRef,
    pub disposition: Disposition,
    pub production_eligible: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ZiplineReceipt {
    pub schema: String,
    pub inputs: ObjectRef,
    pub build: ObjectRef,
    pub ledgers: Ledgers,
    pub bridge: ObjectRef,
    pub differences: ObjectRef,
    pub summary: ObjectRef,
    pub disposition: Disposition,
    pub production_eligible: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ledgers {
    pub targets: ObjectRef,
    pub orders: ObjectRef,
    pub fills: ObjectRef,
    pub positions: ObjectRef,
    pub nav: ObjectRef,
    pub returns: ObjectRef,
    pub costs: ObjectRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerHandle<T> {
    pub receipt: ObjectRef,
    pub artifacts: T,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidationDocument {
    pub schema: String,
    pub job_id: String,
    pub lease_id: String,
    pub primary_job_id: String,
    pub primary_revision: u64,
    pub primary_manifest: ObjectRef,
    pub context_id: String,
    pub policy: ObjectRef,
    pub prepared: ObjectRef,
    pub alphalens: ObjectRef,
    pub zipline: ObjectRef,
    pub disposition: Disposition,
    pub production_eligible: bool,
    pub admission_prerequisites: Vec<String>,
    pub started_at_ms: i64,
}

/// Operation-scoped proof, constructed only after actual supervised execution.
/// It is not serializable and cannot be supplied through RPC or a JSON file.
pub(crate) struct ValidationEvidence {
    pub job: JobSpecification,
    pub primary_record: JobRecord,
    pub primary: Arc<PreparedPortfolio>,
    pub document: ValidationDocument,
    pub report: model::Artifact,
    pub files: Vec<Arc<VerifiedFile>>,
}

impl ValidationEvidence {
    pub(crate) fn check(&self) -> StoreResult<()> {
        let source = self
            .primary_record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("validation primary record"))?;
        self.primary.check(source)?;
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
        self.check()?;
        if record.specification.as_ref() != Some(&self.job) {
            return Err(StoreError::Corrupt("validation job binding"));
        }
        let Some(job_outcome::Outcome::Success(success)) = record
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref())
        else {
            return Err(StoreError::Corrupt("validation completion outcome"));
        };
        if *success != self.success()? {
            return Err(StoreError::Corrupt("validation output binding"));
        }
        Ok(())
    }
}

pub(crate) fn source(job: &JobSpecification) -> StoreResult<(&str, String)> {
    let Some(job_specification::Input::Reconciliation(input)) = &job.input else {
        return Err(StoreError::AdmissionDenied);
    };
    let source = input
        .validation
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    if input.primary_backtest_id.is_some() || input.independent_backtest_id.is_some() {
        return Err(StoreError::Invalid("ambiguous validation source"));
    }
    let id = &source
        .primary_job_id
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?
        .value;
    let context = source
        .context_manifest_sha256
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    if context.value.len() != 32 || job.job_id.as_ref().is_some_and(|job| job.value == *id) {
        return Err(StoreError::Invalid("validation source identity"));
    }
    crate::store::validate_id(id)?;
    Ok((
        id,
        format!(
            "sha256:{}",
            context
                .value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    ))
}
