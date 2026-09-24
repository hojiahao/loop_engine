//! Bound independent receipts. Only the installed runtime constructs authority.

use std::sync::Arc;

use loop_protocol::wire::v1::{
    JobRecord, JobSpecification, JobSuccess, job_outcome, job_specification,
};
use serde::{Deserialize, Serialize};

use super::files::VerifiedFile;
use super::portfolio::{PortfolioWork, PreparedPortfolio};
use super::statistics::StatisticsEvidence;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statistics_job: Option<String>,
}

impl ComparisonPolicy {
    pub(crate) fn validate(&self) -> StoreResult<()> {
        match (&*self.schema, &self.statistics_job) {
            ("loop.reconciliation-policy/v1", None) => {}
            ("loop.reconciliation-policy/v2", Some(id)) => crate::store::validate_id(id)?,
            _ => return Err(StoreError::Invalid("comparison statistics policy")),
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_statistics: Option<StatisticalBinding>,
}

/// Immutable provenance and availability, never an economic acceptance verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatisticalBinding {
    pub job_id: String,
    pub revision: u64,
    pub manifest: ObjectRef,
    pub summary: ObjectRef,
    pub strategy_binding: Option<String>,
    pub available: bool,
    pub reasons: Vec<String>,
}

/// Registered report plus fresh operation-local reconstruction, not caller JSON.
pub(crate) struct RegisteredStatistics {
    pub record: JobRecord,
    pub proof: Arc<StatisticsEvidence>,
}

impl RegisteredStatistics {
    pub(crate) fn binding(&self, primary: &JobRecord) -> StoreResult<StatisticalBinding> {
        self.proof.check_record(&self.record)?;
        let id = primary
            .specification
            .as_ref()
            .and_then(|job| job.job_id.as_ref())
            .ok_or(StoreError::Corrupt("statistical candidate identity"))?;
        if !self
            .proof
            .portfolios
            .iter()
            .any(|(record, _)| record == primary)
        {
            return Err(StoreError::Corrupt(
                "candidate outside statistical population",
            ));
        }
        let summary = &self.proof.summary;
        if summary["schema"] != "loop.global-statistics/v1" {
            return Err(StoreError::Corrupt("global summary schema"));
        }
        let complete = summary["complete_matrix"]
            .as_bool()
            .ok_or(StoreError::Corrupt("global matrix availability"))?;
        let mut reasons = Vec::new();
        let mut strategy_binding = None;
        if complete {
            let strategies = summary["strategies"]
                .as_array()
                .ok_or(StoreError::Corrupt("global strategy set"))?;
            let candidates: Vec<_> = strategies
                .iter()
                .filter(|strategy| {
                    strategy["job_ids"]
                        .as_array()
                        .is_some_and(|jobs| jobs.iter().any(|job| job == &id.value))
                })
                .collect();
            let [candidate] = candidates.as_slice() else {
                return Err(StoreError::Corrupt("global candidate membership"));
            };
            let binding = candidate["binding_sha256"]
                .as_str()
                .ok_or(StoreError::Corrupt("global strategy binding"))?;
            loop_core::factor::ExpressionId::parse(binding)
                .map_err(|_| StoreError::Corrupt("global strategy digest"))?;
            strategy_binding = Some(binding.to_owned());
            // Only interpret producer availability metadata. Numerical tests and
            // economic thresholds belong to research, never this control plane.
            if !candidate["global_by_upper_bound"].is_number() {
                reasons.push("candidate_by_unavailable".to_owned());
            }
            for (name, metric) in [
                ("candidate_dsr_unavailable", &candidate["dsr"]),
                (
                    "global_dsr_benchmark_unavailable",
                    &summary["dsr_benchmark"],
                ),
                ("global_pbo_unavailable", &summary["pbo"]),
            ] {
                if metric["status"] != "available" {
                    reasons.push(name.to_owned());
                }
            }
        } else {
            reasons.push("global_matrix_unavailable".to_owned());
        }
        Ok(StatisticalBinding {
            job_id: self.proof.document.job_id.clone(),
            revision: self.record.revision,
            manifest: self.proof.report.object.clone(),
            summary: self.proof.document.summary.clone(),
            strategy_binding,
            available: reasons.is_empty(),
            reasons,
        })
    }
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
    pub statistics: Option<RegisteredStatistics>,
}

impl ValidationEvidence {
    pub(crate) fn check(&self) -> StoreResult<()> {
        let source = self
            .primary_record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("validation primary record"))?;
        self.primary.check(source)?;
        if self
            .statistics
            .as_ref()
            .map(|statistics| statistics.binding(&self.primary_record))
            .transpose()?
            != self.document.global_statistics
        {
            return Err(StoreError::Corrupt("validation statistics binding"));
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
