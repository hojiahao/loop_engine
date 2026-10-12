//! Operation-local numerical workflow proofs; never transport credentials.

use std::sync::Arc;

use loop_protocol::wire::v1::{JobRecord, job_specification};
use serde::{Deserialize, Serialize};

use crate::manifests::numerical::{DerivedPortfolio, FrozenNumerical};
use crate::manifests::{EvaluationPin, ObjectRef, PortfolioPin};
use crate::store::{BacktestPolicy, EvaluationTrial, StoreError, StoreResult};

mod authority;
pub(crate) use authority::WorkflowAuthority;

/// Frozen recipe/data references copied into each immutable parent transition.
/// Optional pins describe a particular numerical child, not reusable authority.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowEvidence {
    pub schema: String,
    pub recipe: ObjectRef,
    pub data: ObjectRef,
    pub evaluation: Option<EvaluationPin>,
    pub portfolio: Option<PortfolioPin>,
    pub portfolio_context: Option<ObjectRef>,
    pub proposal: Option<ProposalEvidence>,
}

/// Committed model receipt identity; no hidden reasoning or provider credentials.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProposalEvidence {
    pub job_id: String,
    pub ordinal: u32,
    pub request_sha256: String,
    pub response_sha256: String,
    pub ast_sha256: String,
}

impl WorkflowEvidence {
    pub(crate) fn decode(bytes: &[u8]) -> StoreResult<Self> {
        if bytes.is_empty() || bytes.len() > 65_536 {
            return Err(StoreError::Corrupt("workflow evidence size"));
        }
        let evidence: Self = serde_json::from_slice(bytes)
            .map_err(|_| StoreError::Corrupt("workflow evidence encoding"))?;
        if evidence.schema != "loop.run-evidence/v1" {
            return Err(StoreError::Corrupt("workflow evidence schema"));
        }
        evidence.recipe.digest()?;
        evidence.data.digest()?;
        Ok(evidence)
    }

    fn encode(&self) -> StoreResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|_| StoreError::Invalid("workflow evidence encoding"))
    }
}

/// Actual-file proof for starting a v2 run, constructed only by this runtime.
pub(crate) struct PreparedRun {
    frozen: Arc<FrozenNumerical>,
    evidence: Vec<u8>,
}

impl PreparedRun {
    pub(super) fn new(frozen: Arc<FrozenNumerical>) -> StoreResult<Self> {
        frozen.check()?;
        let evidence = WorkflowEvidence {
            schema: "loop.run-evidence/v1".to_owned(),
            recipe: frozen.recipe().clone(),
            data: frozen.data_manifest().clone(),
            evaluation: None,
            portfolio: None,
            portfolio_context: None,
            proposal: None,
        }
        .encode()?;
        Ok(Self { frozen, evidence })
    }

    pub(crate) fn binding(&self) -> &[u8] {
        &self.evidence
    }

    pub(crate) fn check_for(
        &self,
        spec: &loop_protocol::wire::runs::v1::RunSpecification,
    ) -> StoreResult<()> {
        check_recipe(&self.frozen, spec)
    }
}

/// Verified source and proposed child; the store still checks CAS, source state,
/// previous trials and budget atomically. No serialized input can create this proof.
pub(crate) struct PreparedStage {
    frozen: Arc<FrozenNumerical>,
    source: JobRecord,
    trial: Option<EvaluationTrial>,
    input: Option<job_specification::Input>,
    evidence: Vec<u8>,
    portfolio: Option<DerivedPortfolio>,
    child_id: Option<String>,
    source_evaluation: Option<crate::manifests::evaluation::EvaluationEvidence>,
    source_portfolio: Option<Arc<crate::manifests::portfolio::PreparedPortfolio>>,
}

impl PreparedStage {
    pub(super) fn new(
        frozen: Arc<FrozenNumerical>,
        source: JobRecord,
        trial: Option<EvaluationTrial>,
        input: Option<job_specification::Input>,
        evidence: WorkflowEvidence,
        portfolio: Option<DerivedPortfolio>,
    ) -> StoreResult<Self> {
        let child_id = evidence
            .portfolio
            .as_ref()
            .map(|pin| pin.job_id.clone())
            .or_else(|| evidence.evaluation.as_ref().map(|pin| pin.job_id.clone()));
        let prepared = Self {
            frozen,
            source,
            trial,
            input,
            evidence: evidence.encode()?,
            portfolio,
            child_id,
            source_evaluation: None,
            source_portfolio: None,
        };
        prepared.check()?;
        Ok(prepared)
    }

    pub(crate) fn source(&self) -> &JobRecord {
        &self.source
    }
    pub(crate) fn source_trial(&self) -> Option<&EvaluationTrial> {
        self.trial.as_ref()
    }
    pub(crate) fn input(&self) -> Option<&job_specification::Input> {
        self.input.as_ref()
    }
    pub(crate) fn binding(&self) -> &[u8] {
        &self.evidence
    }
    pub(crate) fn child_id(&self) -> Option<&str> {
        self.child_id.as_deref()
    }
    pub(crate) fn check(&self) -> StoreResult<()> {
        self.frozen.check()?;
        if let Some(portfolio) = &self.portfolio {
            portfolio.check()?;
        }
        if let Some(evaluation) = &self.source_evaluation {
            evaluation.check(&self.source)?;
        }
        if let Some(portfolio) = &self.source_portfolio {
            let job = self
                .source
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("source job"))?;
            let Some(loop_protocol::wire::v1::job_outcome::Outcome::Success(success)) = self
                .source
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
            else {
                return Err(StoreError::Corrupt("source outcome"));
            };
            portfolio.resolve_result(job, success)?;
        }
        Ok(())
    }

    pub(super) fn with_evaluation(
        mut self,
        evidence: crate::manifests::evaluation::EvaluationEvidence,
    ) -> StoreResult<Self> {
        self.source_evaluation = Some(evidence);
        self.check()?;
        Ok(self)
    }

    pub(super) fn with_portfolio(
        mut self,
        evidence: Arc<crate::manifests::portfolio::PreparedPortfolio>,
    ) -> StoreResult<Self> {
        self.source_portfolio = Some(evidence);
        self.check()?;
        Ok(self)
    }

    pub(crate) fn check_for(
        &self,
        spec: &loop_protocol::wire::runs::v1::RunSpecification,
    ) -> StoreResult<()> {
        self.check()?;
        if self.source.state == loop_protocol::wire::v1::JobState::Succeeded as i32 {
            match self
                .source
                .specification
                .as_ref()
                .and_then(|job| job.input.as_ref())
            {
                Some(job_specification::Input::FactorEvaluation(_))
                    if self.source_evaluation.is_some() => {}
                Some(job_specification::Input::Backtest(_)) if self.source_portfolio.is_some() => {}
                Some(job_specification::Input::Discovery(_))
                    if WorkflowEvidence::decode(&self.evidence)?.proposal.is_some() => {}
                _ => return Err(StoreError::AdmissionDenied),
            }
        }
        check_recipe(&self.frozen, spec)
    }
}

fn check_recipe(
    frozen: &FrozenNumerical,
    spec: &loop_protocol::wire::runs::v1::RunSpecification,
) -> StoreResult<()> {
    frozen.check()?;
    let digest = spec
        .numerical
        .as_ref()
        .and_then(|workflow| workflow.recipe.as_ref())
        .and_then(|recipe| recipe.sha256.as_ref())
        .ok_or(StoreError::AdmissionDenied)?;
    if digest.value.as_slice() != frozen.recipe().digest()? {
        return Err(StoreError::AdmissionDenied);
    }
    Ok(())
}
