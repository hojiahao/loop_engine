//! Frozen portfolio inputs and verified installed-producer output.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use loop_core::factor::us_equities;
use loop_protocol::wire::v1::{
    Actor, BacktestId, BacktestResult, FactorEvaluationWork, JobSpecification, JobSuccess,
    ResearchProvenanceFingerprint,
};
use prost::Message;
use serde::{Deserialize, Serialize};

use super::files::VerifiedFile;
use super::loading::Materializer;
use super::policy::check_files;
use super::{LocalArtifacts, ObjectRef, model, verification};
use crate::store::{
    AdmissionEvidence, BacktestPolicy, PortfolioLineage, StoreError, StoreResult, TrialLedger,
};

/// Exact server-owned portfolio recipe. The configuration fingerprint must also
/// contain `request`; a deployment pin alone cannot replace frozen inputs.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioPin {
    /// Existing development backtest job.
    pub job_id: String,
    /// Immutable backtest manifest, binding factor, sample, engine and seed.
    pub specification: ObjectRef,
    /// Actual portfolio request, also reached through configuration provenance.
    pub request: ObjectRef,
    /// Registered successful factor evaluation, never an imported result alone.
    pub evaluation_job_id: String,
}

pub(crate) struct PortfolioResolver {
    source: LocalArtifacts,
    registry: Arc<loop_core::factor::OperatorPolicyRegistry>,
    pins: BTreeMap<String, PortfolioPin>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema: String,
    evaluation_work: ObjectRef,
    evaluation_result: ObjectRef,
    factor_values: ObjectRef,
    execution_tape: ObjectRef,
    policies: BTreeMap<String, model::PolicyDocument>,
}

#[derive(Serialize)]
pub(crate) struct PortfolioWork {
    schema: &'static str,
    job_id: String,
    lease_id: String,
    specification: ObjectRef,
    request: ObjectRef,
    trials: TrialLedger,
    manifest: Option<ObjectRef>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortfolioDocument {
    schema: String,
    pub lease_id: String,
    request: ObjectRef,
    pub trials: TrialLedger,
    result: model::ResultManifest,
    supplementary: Vec<model::Artifact>,
}

pub(crate) struct PortfolioInputs {
    job: JobSpecification,
    pub pin: PortfolioPin,
    specification: model::Backtest,
    provenance: ResearchProvenanceFingerprint,
    work: FactorEvaluationWork,
    recipe: Recipe,
    files: Vec<Arc<VerifiedFile>>,
}

pub(crate) struct PreparedPortfolio {
    pub inputs: PortfolioInputs,
    pub result: Option<BacktestResult>,
    pub success: Option<JobSuccess>,
    pub lineage: Option<PortfolioLineage>,
    pub lease_id: Option<String>,
    files: Vec<Arc<VerifiedFile>>,
}

impl PortfolioResolver {
    pub(crate) fn open(root: &Path, pins: Vec<PortfolioPin>) -> StoreResult<Self> {
        if pins.is_empty() || pins.len() > 4096 {
            return Err(StoreError::Invalid("portfolio pin bounds"));
        }
        let mut entries = BTreeMap::new();
        for pin in pins {
            crate::store::validate_id(&pin.job_id)?;
            crate::store::validate_id(&pin.evaluation_job_id)?;
            pin.specification.digest()?;
            pin.request.digest()?;
            if entries.insert(pin.job_id.clone(), pin).is_some() {
                return Err(StoreError::Invalid("duplicate portfolio job"));
            }
        }
        Ok(Self {
            source: LocalArtifacts::open(root)?,
            registry: Arc::new(
                us_equities::registry().map_err(|_| StoreError::Invalid("portfolio registry"))?,
            ),
            pins: entries,
        })
    }

    pub(crate) async fn prepare(&self, job: &JobSpecification) -> StoreResult<PortfolioInputs> {
        let pin = self
            .pins
            .get(
                &job.job_id
                    .as_ref()
                    .ok_or(StoreError::AdmissionDenied)?
                    .value,
            )
            .ok_or(StoreError::AdmissionDenied)?
            .clone();
        let registries = [self.registry.clone()];
        let mut loader = Materializer::for_evaluation(&self.source, &registries);
        let specification: model::Backtest = loader.json(&pin.specification).await?;
        let context = loader.context(&specification.context).await?;
        let factor = loader.factor(&specification.factor, &context).await?;
        verification::frozen_job(job, &specification, &context, &factor)?;
        if specification.engine_version != "authorized-portfolio.1"
            || specification.engine != model::Engine::PrimaryCrossSectional
            || context.portfolio_request.as_ref() != Some(&pin.request)
        {
            return Err(StoreError::Corrupt(
                "portfolio recipe configuration binding",
            ));
        }
        let file = loader.object(&pin.request, true).await?;
        let recipe: Recipe = serde_json::from_slice(file.bytes()?)
            .map_err(|_| StoreError::Corrupt("portfolio request JSON"))?;
        model::schema(&recipe.schema, "loop.portfolio-request/v1")?;
        if recipe.policies.len() != 9 {
            return Err(StoreError::Corrupt("portfolio policy set"));
        }
        for document in recipe.policies.values() {
            if !context.policy_documents.iter().any(|current| {
                current.policy_id == document.policy_id
                    && current.revision == document.revision
                    && current.settings == document.settings
                    && current.schema == document.schema
            }) {
                return Err(StoreError::Corrupt("portfolio actual policies"));
            }
        }
        if !context
            .dataset
            .snapshots
            .iter()
            .flat_map(|snapshot| &snapshot.artifacts)
            .any(|artifact| artifact.object == recipe.execution_tape)
        {
            return Err(StoreError::Corrupt("execution tape outside frozen dataset"));
        }
        let work = FactorEvaluationWork::decode(
            loader
                .object(&recipe.evaluation_work, true)
                .await?
                .bytes()?,
        )
        .map_err(|_| StoreError::Corrupt("portfolio factor work"))?;
        if work.job_id.as_ref().map(|id| id.value.as_str()) != Some(pin.evaluation_job_id.as_str())
            || work
                .factor
                .as_ref()
                .and_then(|factor| factor.factor_spec_id.as_ref())
                .map(|id| id.value.as_str())
                != Some(
                    loop_core::factor::factor_spec_id(&factor)
                        .to_string()
                        .as_str(),
                )
            || work.panel_manifest.as_ref().is_none_or(|panel| {
                !context
                    .dataset
                    .snapshots
                    .iter()
                    .flat_map(|snapshot| &snapshot.artifacts)
                    .any(|artifact| {
                        artifact
                            .wire()
                            .as_ref()
                            .is_ok_and(|reference| reference == panel)
                    })
            })
        {
            return Err(StoreError::Corrupt("portfolio factor identity or panel"));
        }
        loader.object(&recipe.evaluation_result, true).await?;
        loader.object(&recipe.factor_values, false).await?;
        check_files(&loader.files)?;
        Ok(PortfolioInputs {
            job: job.clone(),
            pin,
            specification,
            provenance: context.provenance,
            work,
            recipe,
            files: loader.files,
        })
    }
}

impl PortfolioInputs {
    pub(crate) fn check(&self) -> StoreResult<()> {
        check_files(&self.files)
    }

    pub(crate) fn lineage(&self, trials: TrialLedger) -> PortfolioLineage {
        PortfolioLineage {
            trials,
            work: self.work.clone(),
            evaluation: self.recipe.evaluation_result.clone(),
            values: self.recipe.factor_values.clone(),
        }
    }

    pub(crate) fn work(
        &self,
        lease_id: &str,
        trials: TrialLedger,
        manifest: Option<ObjectRef>,
    ) -> PortfolioWork {
        PortfolioWork {
            schema: "loop.portfolio-work/v1",
            job_id: self.pin.job_id.clone(),
            lease_id: lease_id.to_owned(),
            specification: self.pin.specification.clone(),
            request: self.pin.request.clone(),
            trials,
            manifest,
        }
    }

    pub(crate) fn pending(self) -> PreparedPortfolio {
        PreparedPortfolio {
            inputs: self,
            result: None,
            success: None,
            lineage: None,
            lease_id: None,
            files: vec![],
        }
    }

    pub(crate) async fn seal(
        self,
        outputs: &LocalArtifacts,
        artifact: &model::Artifact,
    ) -> StoreResult<PreparedPortfolio> {
        self.check()?;
        let mut loader = Materializer::for_evaluation(outputs, &[]);
        loader.artifact(artifact).await?;
        let document: PortfolioDocument = loader.json(&artifact.object).await?;
        let result = &document.result;
        model::schema(&document.schema, "loop.authorized-portfolio/v1")?;
        model::schema(&result.schema, "loop.backtest-result/v1")?;
        crate::store::validate_id(&document.lease_id)?;
        if artifact.schema.name != "loop.authorized_portfolio"
            || artifact.schema.version != 1
            || result.job_id != self.pin.job_id
            || result.specification != self.pin.specification
            || document.request != self.pin.request
            || result.engine != self.specification.engine
            || result.engine_version != self.specification.engine_version
            || artifact.created_at_ms != result.completed_at_ms
            || document.supplementary.len() != 6
        {
            return Err(StoreError::Corrupt("portfolio result envelope"));
        }
        let mut success = JobSuccess { outputs: vec![] };
        for entry in result
            .artifacts
            .entries()
            .into_iter()
            .chain(&document.supplementary)
            .chain([artifact])
        {
            loader.artifact(entry).await?;
            success.outputs.push(entry.wire()?);
        }
        let result = BacktestResult {
            backtest_id: Some(BacktestId {
                value: self.specification.backtest_id.clone(),
            }),
            engine: result.engine.wire(),
            engine_version: result.engine_version.clone(),
            provenance: Some(self.provenance.clone()),
            metrics: result
                .metrics
                .iter()
                .map(model::Metric::wire)
                .collect::<StoreResult<_>>()?,
            artifacts: Some(result.artifacts.wire()?),
            result_manifest_sha256: Some(model::digest(artifact.object.digest()?)),
            completed_at: Some(model::timestamp(result.completed_at_ms)?),
        };
        let lineage = self.lineage(document.trials);
        check_files(&loader.files)?;
        Ok(PreparedPortfolio {
            inputs: self,
            result: Some(result),
            success: Some(success),
            lineage: Some(lineage),
            lease_id: Some(document.lease_id),
            files: loader.files,
        })
    }
}

impl PreparedPortfolio {
    pub(crate) fn check(&self, job: &JobSpecification) -> StoreResult<()> {
        if job != &self.inputs.job {
            return Err(StoreError::Corrupt("prepared portfolio job"));
        }
        self.inputs.check()?;
        check_files(&self.files)
    }
}

impl BacktestPolicy for PreparedPortfolio {
    fn validate_inputs(&self, job: &JobSpecification) -> StoreResult<()> {
        self.check(job)
    }

    fn resolve_result(
        &self,
        job: &JobSpecification,
        success: &JobSuccess,
    ) -> StoreResult<BacktestResult> {
        self.check(job)?;
        if self.success.as_ref() != Some(success) {
            return Err(StoreError::Corrupt("portfolio success binding"));
        }
        self.result.clone().ok_or(StoreError::AdmissionDenied)
    }

    fn portfolio_lineage(&self, job: &JobSpecification) -> StoreResult<Option<PortfolioLineage>> {
        self.check(job)?;
        Ok(self.lineage.clone())
    }

    fn resolve_current(
        &self,
        _: &Actor,
        job: &JobSpecification,
        context: &str,
    ) -> StoreResult<Option<ResearchProvenanceFingerprint>> {
        self.check(job)?;
        if context != self.inputs.specification.context.sha256 {
            return Err(StoreError::Unavailable("unregistered portfolio context"));
        }
        Ok(Some(self.inputs.provenance.clone()))
    }

    fn resolve_admission(
        &self,
        _: &Actor,
        job: &JobSpecification,
        context: &str,
    ) -> StoreResult<AdmissionEvidence> {
        self.resolve_current(&Actor::default(), job, context)?;
        // The shared command has already verified current registered numerical
        // lineage. Phase 8 evidence is an unresolved dependency, never a factor
        // rejection or a semantic exception that force could waive.
        Err(StoreError::IndependentPending)
    }
}
