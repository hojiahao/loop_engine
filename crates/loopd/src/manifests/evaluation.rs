use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Datelike;
use loop_core::factor::{FactorSpecId, ValidationLimits, parse_factor_spec, us_equities};
use loop_protocol::job::{factor_identity_bytes, validate_factor_identity};
use loop_protocol::provenance::ProvenanceSnapshot;
use loop_protocol::wire::v1::{
    ArtifactRef, CivilDate, FactorEvaluationResult, FactorEvaluationWork, JobRecord,
    JobSpecification, JobSuccess, LeaseId, ResearchProvenanceFingerprint, job_outcome,
    job_specification,
};
use serde::{Deserialize, Serialize};

use super::files::{ReadBudget, VerifiedFile};
use super::loading::{Materializer, policies};
use super::policy::check_files;
use super::{LocalArtifacts, ObjectRef, model};
use crate::store::{StoreError, StoreResult};

mod transform;
use transform::{BoundTransform, PanelTransform, TransformEvidence};

/// Deployment pin for one frozen development-evaluation context.
/// A pin is not transport authority and cannot introduce protected samples.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationPin {
    /// Exact existing durable job ID.
    pub job_id: String,
    /// Content-addressed six-component research context.
    pub context: ObjectRef,
}

/// Actual-file resolver for the closed raw/transformed numerical execution paths.
/// Resolves before computation, retains guards and denies missing/drifting data.
pub struct EvaluationResolver {
    artifacts: LocalArtifacts,
    registry: Arc<loop_core::factor::OperatorPolicyRegistry>,
    pins: BTreeMap<String, ObjectRef>,
}

pub(crate) struct EvaluationInputs {
    job: JobSpecification,
    pub work: FactorEvaluationWork,
    quality: model::Quality,
    transform: Option<BoundTransform>,
    minimum_coverage_bps: u32,
    files: Vec<Arc<VerifiedFile>>,
}

/// Crate-private completion proof, constructed only from actual resolved files.
/// A caller's result hash or ArtifactRef cannot construct transaction authority.
pub(crate) struct EvaluationEvidence {
    job: JobSpecification,
    pub result: FactorEvaluationResult,
    pub minimum_coverage_bps: u32,
    files: Vec<Arc<VerifiedFile>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanelDocument {
    schema: String,
    quality: model::Quality,
    sessions: Vec<String>,
    securities: Vec<String>,
    fields: Vec<String>,
    decision_times_ms: Vec<i64>,
    evaluation_start: String,
    values: ObjectRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transform: Option<PanelTransform>,
}

impl EvaluationResolver {
    /// Open an absolute deployment-owned development object store and exact pins.
    /// No directory scan discovers trust. No database mutation or worker launch
    /// occurs; transport authorization must precede `prepare` at the runtime.
    pub fn open(root: &std::path::Path, pins: Vec<EvaluationPin>) -> StoreResult<Self> {
        if pins.is_empty() || pins.len() > 4096 {
            return Err(StoreError::Invalid("evaluation registry bounds"));
        }
        let mut contexts = BTreeMap::new();
        for pin in pins {
            crate::store::validate_id(&pin.job_id)?;
            pin.context.digest()?;
            if contexts.insert(pin.job_id, pin.context).is_some() {
                return Err(StoreError::Invalid("duplicate evaluation job"));
            }
        }
        Ok(Self {
            artifacts: LocalArtifacts::open(root)?,
            registry: Arc::new(
                us_equities::registry()
                    .map_err(|_| StoreError::Invalid("installed operator registry"))?,
            ),
            pins: contexts,
        })
    }

    pub(crate) async fn prepare(
        &self,
        job: &JobSpecification,
        lease: &LeaseId,
    ) -> StoreResult<EvaluationInputs> {
        let Some(job_specification::Input::FactorEvaluation(input)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        let factor = input.factor.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let provenance = input
            .provenance
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        ProvenanceSnapshot::try_from(provenance)?;
        let seed = input
            .deterministic_seed
            .as_ref()
            .filter(|seed| seed.value.len() == 32)
            .ok_or(StoreError::AdmissionDenied)?;
        let context = self
            .pins
            .get(
                &job.job_id
                    .as_ref()
                    .ok_or(StoreError::AdmissionDenied)?
                    .value,
            )
            .ok_or(StoreError::AdmissionDenied)?;
        let registries = [self.registry.clone()];
        let mut loader = Materializer::for_evaluation(&self.artifacts, &registries);
        let resolved = loader.context(context).await?;
        if provenance != &resolved.provenance
            || input.dataset.as_ref() != Some(&resolved.dataset.reference(&resolved.manifest.data)?)
            || !matches!(
                resolved.engine_version.as_str(),
                "factor-evaluator.2" | "factor-evaluator.3"
            )
            || resolved.engine != model::Engine::PrimaryCrossSectional
            || !matches!(
                resolved.dataset.sample.role,
                model::SampleRole::InSample | model::SampleRole::DevelopmentValidation
            )
        {
            return Err(StoreError::Corrupt("frozen evaluation context"));
        }
        validate_factor_identity(factor)?;
        let canonical = parse_factor_spec(
            &factor_identity_bytes(factor)?,
            FactorSpecId::parse(
                &factor
                    .factor_spec_id
                    .as_ref()
                    .ok_or(StoreError::AdmissionDenied)?
                    .value,
            )
            .map_err(|_| StoreError::Invalid("factor ID"))?,
            &factor
                .expression
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?
                .canonical_json,
            &self.registry,
            ValidationLimits::default(),
        )
        .map_err(|_| StoreError::Invalid("canonical executable factor"))?;
        for policy in policies(&canonical) {
            if !resolved.policies.iter().any(|entry| {
                entry.policy_id == policy.policy_id().as_str()
                    && entry.revision == policy.revision().as_str()
                    && entry
                        .document
                        .digest()
                        .is_ok_and(|digest| &digest == policy.sha256())
            }) {
                return Err(StoreError::Corrupt("evaluation policy binding"));
            }
        }
        let evaluation_policy = resolved
            .policy_documents
            .iter()
            .find(|document| {
                document.policy_id == canonical.evaluation_policy().policy_id().as_str()
            })
            .ok_or(StoreError::Corrupt("evaluation coverage policy"))?;
        let minimum_coverage_bps = evaluation_policy
            .settings
            .get("minimum_coverage_bps")
            .and_then(|value| {
                value.parse::<u32>().ok().filter(|minimum| {
                    (1..=10_000).contains(minimum) && minimum.to_string() == *value
                })
            })
            .ok_or(StoreError::Invalid("frozen minimum coverage"))?;
        let panels: Vec<_> = resolved
            .dataset
            .snapshots
            .iter()
            .filter(|snapshot| {
                snapshot
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.schema.name == "loop.factor_panel")
            })
            .collect();
        if panels.len() != 1 {
            return Err(StoreError::Invalid(
                "evaluation requires one explicit panel snapshot",
            ));
        }
        // Additional frozen execution snapshots may accompany this panel. The
        // evaluator only consumes panel artifacts; the portfolio worker later
        // reads execution observations through the same authorized data view.
        let snapshot = panels[0];
        let artifact = snapshot
            .artifacts
            .iter()
            .find(|artifact| {
                artifact.schema.name == "loop.factor_panel"
                    && matches!(artifact.schema.version, 1 | 2)
                    && artifact.media_type == "application/json"
            })
            .ok_or(StoreError::Invalid("evaluation panel manifest"))?;
        let panel: PanelDocument = loader.json(&artifact.object).await?;
        let transformation = match (
            panel.schema.as_str(),
            artifact.schema.version,
            panel.transform.as_ref(),
            resolved.engine_version.as_str(),
        ) {
            ("loop.factor-panel/v1", 1, None, "factor-evaluator.2") => {
                for expected in [
                    canonical.preprocess_policy(),
                    canonical.neutralization_policy(),
                ] {
                    if !resolved.policy_documents.iter().any(|document| {
                        document.policy_id == expected.policy_id().as_str()
                            && document.revision == expected.revision().as_str()
                            && document.settings.is_empty()
                    }) {
                        return Err(StoreError::Invalid(
                            "raw evaluator cannot ignore transformation policy",
                        ));
                    }
                }
                None
            }
            ("loop.factor-panel/v2", 2, Some(transformation), "factor-evaluator.3") => {
                let start = panel
                    .sessions
                    .iter()
                    .position(|day| day == &panel.evaluation_start)
                    .ok_or(StoreError::Invalid("transformation evaluation boundary"))?;
                Some(transformation.bind(&canonical, panel.sessions.len() - start)?)
            }
            _ => return Err(StoreError::Invalid("evaluation panel/engine version")),
        };
        let exposure = transformation
            .as_ref()
            .and_then(|value| value.exposures.as_ref());
        if snapshot.artifacts.len() != 2 + usize::from(exposure.is_some())
            || exposure.is_some_and(|reference| {
                !snapshot.artifacts.iter().any(|entry| {
                    entry.object == *reference
                        && entry.schema.name == "loop.factor_exposures"
                        && entry.schema.version == 1
                        && entry.media_type == "text/csv"
                })
            })
        {
            return Err(StoreError::Corrupt("transformation exposure artifact set"));
        }
        let calendar: model::Calendar = loader.json(&resolved.manifest.calendar).await?;
        if panel.sessions != calendar.sessions
            || panel.quality != resolved.dataset.quality
            || panel.decision_times_ms.len() != panel.sessions.len()
            || panel
                .decision_times_ms
                .iter()
                .any(|time| *time > snapshot.known_through_ms)
            || !snapshot.artifacts.iter().any(|entry| {
                entry.object == panel.values
                    && entry.schema.name == "loop.factor_panel_values"
                    && entry.schema.version == 1
                    && entry.media_type == "text/csv"
            })
        {
            return Err(StoreError::Corrupt("panel/calendar/data binding"));
        }
        check_files(&loader.files)?;
        Ok(EvaluationInputs {
            job: job.clone(),
            work: FactorEvaluationWork {
                job_id: job.job_id.clone(),
                lease_id: Some(lease.clone()),
                factor: Some(factor.clone()),
                panel_manifest: Some(artifact.wire()?),
                sample_start: Some(civil(&resolved.dataset.sample.start)?),
                sample_end: Some(civil(&resolved.dataset.sample.end)?),
                provenance: Some(provenance.clone()),
                deterministic_seed: Some(seed.clone()),
            },
            quality: resolved.dataset.quality,
            transform: transformation,
            minimum_coverage_bps,
            files: loader.files,
        })
    }
}

impl EvaluationInputs {
    pub(crate) fn check(&self) -> StoreResult<()> {
        check_files(&self.files)
    }

    pub(crate) async fn resolve(
        self,
        outputs: &LocalArtifacts,
        success: &JobSuccess,
    ) -> StoreResult<EvaluationEvidence> {
        self.check()?;
        if success.outputs.len() != 2 {
            return Err(StoreError::Corrupt("evaluation output set"));
        }
        let values = &success.outputs[0];
        let manifest = &success.outputs[1];
        let mut files = self.files;
        let mut budget = ReadBudget::new();
        let output_version = if self.transform.is_some() { 2 } else { 1 };
        for (artifact, name, media_type, columns) in [
            (
                values,
                "loop.factor_values",
                "text/csv",
                vec!["session", "security_id", "eligible", "value"],
            ),
            (
                manifest,
                "loop.factor_evaluation",
                "application/json",
                vec![
                    "factor_spec_id",
                    "values_sha256",
                    "valid_observations",
                    "eligible_observations",
                ],
            ),
        ] {
            loop_protocol::artifact::validate_artifact_ref(artifact)
                .map_err(|_| StoreError::Corrupt("evaluation artifact"))?;
            let schema = model::SchemaDocument {
                schema: "loop.artifact-schema/v1".to_owned(),
                name: name.to_owned(),
                version: output_version,
                media_type: media_type.to_owned(),
                columns: columns.into_iter().map(str::to_owned).collect(),
            };
            let bytes = serde_json::to_vec(&schema)
                .map_err(|_| StoreError::Corrupt("evaluation schema"))?;
            use sha2::Digest;
            let digest = sha2::Sha256::digest(&bytes).to_vec();
            if artifact.schema.as_ref().is_none_or(|schema| {
                schema.name != name
                    || schema.version != output_version
                    || schema
                        .schema_sha256
                        .as_ref()
                        .is_none_or(|hash| hash.value != digest)
            }) || artifact.media_type != media_type
                || artifact.byte_size > 64 * 1024 * 1024
            {
                return Err(StoreError::Corrupt("evaluation artifact schema"));
            }
            let schema_ref = ObjectRef {
                sha256: format!(
                    "sha256:{}",
                    digest
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                ),
                byte_size: bytes.len() as u64,
            };
            files.push(outputs.load(&schema_ref, true, &mut budget).await?);
            files.push(
                outputs
                    .load(
                        &reference(artifact)?,
                        name == "loop.factor_evaluation",
                        &mut budget,
                    )
                    .await?,
            );
        }
        let document: ReportDocument = files
            .last()
            .ok_or(StoreError::Corrupt("result manifest"))?
            .json()?;
        let result = document.result(values.clone(), manifest.clone())?;
        match (&self.transform, &document.transform) {
            (None, None) => {}
            (Some(expected), Some(actual)) => expected.check(
                actual,
                result.eligible_observations,
                result.valid_observations,
            )?,
            _ => return Err(StoreError::Corrupt("transformation result version")),
        }
        let work = &self.work;
        let factor = work
            .factor
            .as_ref()
            .ok_or(StoreError::Corrupt("evaluation factor"))?;
        if result.job_id != work.job_id
            || result.lease_id != work.lease_id
            || result.factor_spec_id != factor.factor_spec_id
            || result.expression_id != factor.expression_id
            || result.provenance != work.provenance
            || result.deterministic_seed != work.deterministic_seed
            || result.sample_start != work.sample_start
            || result.sample_end != work.sample_end
            || document.panel_manifest_sha256
                != reference(
                    work.panel_manifest
                        .as_ref()
                        .ok_or(StoreError::Corrupt("panel"))?,
                )?
                .sha256
            || document.quality != self.quality
            || document.values_sha256 != reference(values)?.sha256
            || result.valid_observations > result.eligible_observations
            || result.eligible_observations > values.row_count.unwrap_or(0)
            || values
                .row_count
                .is_none_or(|count| count == 0 || count > 2_000_000)
            || result.work_units == 0
            || result.work_units > 50_000_000
            || result.completed_at != values.created_at
            || result.completed_at != manifest.created_at
        {
            return Err(StoreError::Corrupt("numerical result frozen binding"));
        }
        check_files(&files)?;
        Ok(EvaluationEvidence {
            job: self.job,
            result,
            minimum_coverage_bps: self.minimum_coverage_bps,
            files,
        })
    }
}

impl EvaluationEvidence {
    pub(crate) fn check(&self, record: &JobRecord) -> StoreResult<()> {
        check_files(&self.files)?;
        let expected = self.success()?;
        if record.specification.as_ref() != Some(&self.job)
            || !matches!(record.outcome.as_ref().and_then(|outcome| outcome.outcome.as_ref()),
                Some(job_outcome::Outcome::Success(success)) if success == &expected)
        {
            return Err(StoreError::Corrupt("evaluation completion binding"));
        }
        let completed = self
            .result
            .completed_at
            .as_ref()
            .ok_or(StoreError::Corrupt("evaluation time"))?;
        let millis =
            |time: &prost_types::Timestamp| time.seconds * 1000 + i64::from(time.nanos) / 1_000_000;
        if self
            .job
            .submitted_at
            .as_ref()
            .is_none_or(|time| millis(time) > millis(completed))
            || record
                .updated_at
                .as_ref()
                .is_none_or(|time| millis(time) < millis(completed))
        {
            return Err(StoreError::Corrupt("evaluation completion time"));
        }
        Ok(())
    }

    pub(crate) fn success(&self) -> StoreResult<JobSuccess> {
        Ok(JobSuccess {
            outputs: vec![
                self.result
                    .values
                    .clone()
                    .ok_or(StoreError::Corrupt("evaluation values"))?,
                self.result
                    .manifest
                    .clone()
                    .ok_or(StoreError::Corrupt("evaluation manifest"))?,
            ],
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceDocument {
    source_code_sha256: String,
    operator_registry_sha256: String,
    configuration_sha256: String,
    data_manifest_sha256: String,
    trading_calendar_sha256: String,
    environment_sha256: String,
}

impl ProvenanceDocument {
    fn wire(&self) -> StoreResult<ResearchProvenanceFingerprint> {
        let hash = |value: &str| model::parse_digest(value).map(model::digest);
        Ok(ResearchProvenanceFingerprint {
            source_code_sha256: Some(hash(&self.source_code_sha256)?),
            operator_registry_sha256: Some(hash(&self.operator_registry_sha256)?),
            configuration_sha256: Some(hash(&self.configuration_sha256)?),
            data_manifest_sha256: Some(hash(&self.data_manifest_sha256)?),
            trading_calendar_sha256: Some(hash(&self.trading_calendar_sha256)?),
            environment_sha256: Some(hash(&self.environment_sha256)?),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportDocument {
    schema: String,
    job_id: String,
    lease_id: String,
    factor_spec_id: String,
    expression_id: String,
    provenance: ProvenanceDocument,
    deterministic_seed: String,
    panel_manifest_sha256: String,
    values_sha256: String,
    quality: model::Quality,
    sample_start: String,
    sample_end: String,
    eligible_observations: u64,
    valid_observations: u64,
    work_units: u64,
    completed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transform: Option<TransformEvidence>,
}

impl ReportDocument {
    fn result(
        &self,
        values: ArtifactRef,
        manifest: ArtifactRef,
    ) -> StoreResult<FactorEvaluationResult> {
        model::schema(
            &self.schema,
            if self.transform.is_some() {
                "loop.factor-evaluation-result/v2"
            } else {
                "loop.factor-evaluation-result/v1"
            },
        )?;
        Ok(FactorEvaluationResult {
            job_id: Some(loop_protocol::wire::v1::JobId {
                value: self.job_id.clone(),
            }),
            lease_id: Some(LeaseId {
                value: self.lease_id.clone(),
            }),
            factor_spec_id: Some(loop_protocol::wire::v1::FactorSpecId {
                value: self.factor_spec_id.clone(),
            }),
            expression_id: Some(loop_protocol::wire::v1::FactorExpressionId {
                value: self.expression_id.clone(),
            }),
            provenance: Some(self.provenance.wire()?),
            deterministic_seed: Some(model::digest(model::parse_digest(
                &self.deterministic_seed,
            )?)),
            values: Some(values),
            eligible_observations: self.eligible_observations,
            valid_observations: self.valid_observations,
            work_units: self.work_units,
            sample_start: Some(civil(&self.sample_start)?),
            sample_end: Some(civil(&self.sample_end)?),
            completed_at: Some(model::timestamp(self.completed_at_ms)?),
            manifest: Some(manifest),
        })
    }
}

fn civil(value: &str) -> StoreResult<CivilDate> {
    let date = model::date(value)?;
    Ok(CivilDate {
        year: date.year(),
        month: date.month(),
        day: date.day(),
    })
}

fn reference(artifact: &ArtifactRef) -> StoreResult<ObjectRef> {
    Ok(ObjectRef {
        sha256: artifact
            .artifact_id
            .as_ref()
            .ok_or(StoreError::Corrupt("artifact ID"))?
            .value
            .clone(),
        byte_size: artifact.byte_size,
    })
}
