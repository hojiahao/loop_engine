use std::collections::BTreeSet;
use std::sync::Arc;

use loop_core::factor::{FactorSpec, OperatorPolicyRegistry, factor_spec_id};
use loop_protocol::wire::v1::{
    Actor, ActorKind, ArtifactRef, BacktestResult, JobSpecification, JobSuccess, PerturbationSpace,
    ResearchProvenanceFingerprint, job_outcome, job_specification,
};
use serde::Serialize;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::files::VerifiedFile;
use super::loading::{Materializer, ResolvedContext, sorted};
use super::{LocalArtifacts, ObjectRef, model, verification};
use crate::store::{
    AdmissionEvidence, BacktestExport, BacktestPolicy, BacktestPreparation, BacktestRepository,
    ExportBacktest, JobRepository, PgJobStore, StoreError, StoreResult,
};

/// Concrete immutable-file resolver for explicitly registered development work.
/// A service pins the catalog and readers; a request cannot add trust or grant
/// itself access. The transport must independently authenticate `Actor` values.
/// Protected samples and human overrides remain denied. Loading this resolver
/// does not register a public endpoint or attest numerical/data quality.
pub struct TrustedManifests {
    artifacts: LocalArtifacts,
    catalog: model::Catalog,
    catalog_file: Arc<VerifiedFile>,
    registries: Vec<Arc<OperatorPolicyRegistry>>,
    readers: Arc<Vec<Actor>>,
}

struct Prepared {
    catalog_file: Arc<VerifiedFile>,
    readers: Arc<Vec<Actor>>,
    frozen: Arc<Frozen>,
    current: Option<(String, Arc<Current>)>,
}

struct Frozen {
    job: JobSpecification,
    specification: model::Backtest,
    context: ResolvedContext,
    factor: FactorSpec,
    result: Option<(BacktestResult, model::ResultManifest, ArtifactRef)>,
    review: Option<AdmissionEvidence>,
    files: Vec<Arc<VerifiedFile>>,
}

struct Current {
    context: ResolvedContext,
    family: Option<PerturbationSpace>,
    files: Vec<Arc<VerifiedFile>>,
}

impl TrustedManifests {
    /// Load a service-pinned catalog, not a caller-selected manifest or URI.
    /// Registries have already passed `loop-core` semantic-contract validation;
    /// their canonical bytes must also match the context's actual file. Readers
    /// are service-assigned identities authorized to every development object
    /// and reviewed library in this catalog. Empty/invalid collections deny.
    /// Loading is read-only, bounded and cancellation-safe; no database changes.
    pub async fn load(
        artifacts: LocalArtifacts,
        catalog: ObjectRef,
        registries: Vec<Arc<OperatorPolicyRegistry>>,
        readers: Vec<Actor>,
    ) -> StoreResult<Self> {
        if registries.is_empty()
            || registries.len() > 16
            || readers.is_empty()
            || readers.len() > 128
        {
            return Err(StoreError::Invalid("manifest deployment bounds"));
        }
        let registry_ids = registries
            .iter()
            .map(|registry| registry.identity())
            .collect::<BTreeSet<_>>();
        if registry_ids.len() != registries.len() {
            return Err(StoreError::Invalid("duplicate deployment registry"));
        }
        let mut subjects = BTreeSet::new();
        for reader in &readers {
            if reader
                .actor_id
                .as_ref()
                .is_none_or(|id| id.value.is_empty())
                || reader.authenticated_subject.is_empty()
                || !matches!(
                    ActorKind::try_from(reader.kind),
                    Ok(ActorKind::Human
                        | ActorKind::Service
                        | ActorKind::Agent
                        | ActorKind::Scheduler)
                )
                || !subjects.insert(reader.authenticated_subject.as_str())
            {
                return Err(StoreError::Invalid("manifest reader identity"));
            }
        }
        let mut loader = Materializer::new(&artifacts, &registries);
        let catalog_file = loader.object(&catalog, true).await?;
        let parsed: model::Catalog = catalog_file.json()?;
        model::schema(&parsed.schema, "loop.research-catalog/v1")?;
        sorted(
            parsed
                .contexts
                .iter()
                .map(|entry| entry.context_id.as_str()),
            1,
            128,
        )?;
        sorted(
            parsed.backtests.iter().map(|entry| entry.job_id.as_str()),
            1,
            128,
        )?;
        for entry in &parsed.contexts {
            entry.manifest.digest()?;
            if entry.context_id != entry.manifest.sha256 {
                return Err(StoreError::Invalid(
                    "current context must be content-addressed",
                ));
            }
        }
        for entry in &parsed.backtests {
            entry.specification.digest()?;
            if let Some(result) = &entry.result {
                result.wire()?;
            }
            if let Some(review) = &entry.review {
                review.wire()?;
            }
        }
        Ok(Self {
            artifacts,
            catalog: parsed,
            catalog_file,
            registries,
            readers: Arc::new(readers),
        })
    }

    fn authorize(&self, principal: &Actor) -> StoreResult<()> {
        if !self.readers.contains(principal) {
            return Err(StoreError::AdmissionDenied);
        }
        self.catalog_file.check()
    }
}

impl Prepared {
    fn authorize(&self, principal: &Actor) -> StoreResult<()> {
        if !self.readers.contains(principal) {
            return Err(StoreError::AdmissionDenied);
        }
        self.catalog_file.check()
    }

    fn frozen(&self, job: &JobSpecification) -> StoreResult<Arc<Frozen>> {
        self.catalog_file.check()?;
        let frozen = Arc::clone(&self.frozen);
        if frozen.job != *job {
            return Err(StoreError::Corrupt("prepared job binding"));
        }
        check_files(&frozen.files)?;
        verification::frozen_job(job, &frozen.specification, &frozen.context, &frozen.factor)?;
        Ok(frozen)
    }

    fn current(&self, principal: &Actor, id: &str) -> StoreResult<Arc<Current>> {
        self.authorize(principal)?;
        let current = self
            .current
            .as_ref()
            .filter(|(context_id, _)| context_id == id)
            .map(|(_, context)| Arc::clone(context))
            .ok_or(StoreError::Unavailable("unprepared current context"))?;
        check_files(&current.files)?;
        Ok(current)
    }
}

impl TrustedManifests {
    async fn materialize(
        &self,
        job: &JobSpecification,
        context: Option<&str>,
        success: Option<&JobSuccess>,
    ) -> StoreResult<Prepared> {
        self.catalog_file.check()?;
        if !matches!(job.input, Some(job_specification::Input::Backtest(_))) {
            return Err(StoreError::AdmissionDenied);
        }
        let entry = self
            .catalog
            .backtests
            .iter()
            .find(|entry| {
                Some(entry.job_id.as_str()) == job.job_id.as_ref().map(|id| id.value.as_str())
            })
            .ok_or(StoreError::AdmissionDenied)?;
        let mut loader = Materializer::new(&self.artifacts, &self.registries);
        let specification: model::Backtest = loader.json(&entry.specification).await?;
        let resolved = loader.context(&specification.context).await?;
        let factor = loader.factor(&specification.factor, &resolved).await?;
        verification::frozen_job(job, &specification, &resolved, &factor)?;
        let mut result = None;
        let mut review = None;
        if let Some(success) = success {
            let artifact = entry
                .result
                .as_ref()
                .ok_or(StoreError::Unavailable("unregistered result manifest"))?;
            let (wire, document) = loader
                .result(artifact, entry, &specification, &resolved)
                .await?;
            let manifest = artifact.wire()?;
            verify_outputs(success, &document, &manifest)?;
            result = Some((wire, document, manifest));
            review = verification::review(&mut loader, entry, &resolved, &factor).await?;
        }
        let frozen = Arc::new(Frozen {
            job: job.clone(),
            specification,
            context: resolved,
            factor,
            result,
            review,
            files: loader.files,
        });
        check_files(&frozen.files)?;
        let mut current = None;
        if let Some(id) = context {
            let entry = self
                .catalog
                .contexts
                .iter()
                .find(|entry| entry.context_id == id)
                .ok_or(StoreError::Unavailable("unregistered current context"))?;
            let mut loader = Materializer::new(&self.artifacts, &self.registries);
            let context = loader.context(&entry.manifest).await?;
            let family = verification::family(&mut loader, &context).await?;
            let evidence = Arc::new(Current {
                context,
                family,
                files: loader.files,
            });
            check_files(&evidence.files)?;
            current = Some((id.to_owned(), evidence));
        }
        self.catalog_file.check()?;
        Ok(Prepared {
            catalog_file: Arc::clone(&self.catalog_file),
            readers: Arc::clone(&self.readers),
            frozen,
            current,
        })
    }

    /// Write current result JSON only after PostgreSQL commits its export
    /// receipt/audit. Rechecks actual file guards and result equality immediately
    /// before writing. The writer belongs to an authenticated host handler, not
    /// an arbitrary Agent path. No data-series bytes or holdout material pass.
    /// Writer failure may leave a partial destination and an accepted metadata
    /// receipt; retry reauthorizes and never claims exactly-once file delivery.
    pub async fn write_current(
        &self,
        store: &PgJobStore,
        principal: &Actor,
        command: ExportBacktest,
        writer: &mut (impl AsyncWrite + Unpin + Send),
    ) -> StoreResult<BacktestExport> {
        self.authorize(principal)?;
        let export = store.export_current(principal, command).await?;
        let record = store
            .get(&export.job_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("export source"))?;
        let success = match record
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref())
        {
            Some(job_outcome::Outcome::Success(success)) => success,
            _ => return Err(StoreError::Corrupt("export source outcome")),
        };
        self.authorize_materialization(principal, job, Some(&export.context_id))?;
        let prepared = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.materialize(job, Some(&export.context_id), Some(success)),
        )
        .await
        .map_err(|_| StoreError::Unavailable("export verification timeout"))??;
        let current = prepared.current(principal, &export.context_id)?;
        let frozen = prepared.frozen(job)?;
        check_files(&frozen.files)?;
        let (result, manifest, _) = frozen
            .result
            .as_ref()
            .ok_or(StoreError::Unavailable("unprepared result"))?;
        if result != &export.result
            || result.provenance.as_ref() != Some(&current.context.provenance)
        {
            return Err(StoreError::Corrupt("export evidence binding"));
        }
        #[derive(Serialize)]
        struct Export<'a> {
            schema: &'static str,
            job_id: &'a str,
            context_id: &'a str,
            accepted_at_ms: i64,
            replayed: bool,
            data_quality: model::Quality,
            return_definition: &'a str,
            result_manifest: &'a model::ResultManifest,
        }
        let bytes = serde_json::to_vec(&Export {
            schema: "loop.current-backtest-export/v1",
            job_id: &export.job_id,
            context_id: &export.context_id,
            accepted_at_ms: export.accepted_at.seconds * 1000
                + i64::from(export.accepted_at.nanos) / 1_000_000,
            replayed: export.replayed,
            data_quality: current.context.dataset.quality,
            return_definition: &frozen.specification.return_definition,
            result_manifest: manifest,
        })
        .map_err(|_| StoreError::Corrupt("export JSON"))?;
        check_files(&frozen.files)?;
        check_files(&current.files)?;
        self.catalog_file.check()?;
        if bytes.len() > 2_097_152 {
            return Err(StoreError::Invalid("export byte limit"));
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            writer
                .write_all(&bytes)
                .await
                .map_err(|_| StoreError::Unavailable("export writer"))?;
            writer
                .flush()
                .await
                .map_err(|_| StoreError::Unavailable("export flush"))
        })
        .await
        .map_err(|_| StoreError::Unavailable("export delivery timeout"))??;
        Ok(export)
    }
}

impl BacktestPolicy for TrustedManifests {
    fn authorize_materialization(
        &self,
        principal: &Actor,
        job: &JobSpecification,
        context: Option<&str>,
    ) -> StoreResult<()> {
        self.authorize(principal)?;
        if !self.catalog.backtests.iter().any(|entry| {
            Some(entry.job_id.as_str()) == job.job_id.as_ref().map(|id| id.value.as_str())
        }) || context.is_some_and(|id| {
            !self
                .catalog
                .contexts
                .iter()
                .any(|entry| entry.context_id == id)
        }) {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }

    fn prepare<'a>(
        &'a self,
        job: &'a JobSpecification,
        context: Option<&'a str>,
        success: Option<&'a JobSuccess>,
    ) -> BacktestPreparation<'a> {
        Box::pin(async move {
            let prepared = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                self.materialize(job, context, success),
            )
            .await
            .map_err(|_| StoreError::Unavailable("research materialization timeout"))??;
            Ok(Some(Arc::new(prepared) as Arc<dyn BacktestPolicy>))
        })
    }

    fn validate_inputs(&self, _: &JobSpecification) -> StoreResult<()> {
        Err(StoreError::Unavailable("unprepared research inputs"))
    }
}

impl BacktestPolicy for Prepared {
    fn validate_inputs(&self, job: &JobSpecification) -> StoreResult<()> {
        self.frozen(job).map(|_| ())
    }

    fn validate_worker(
        &self,
        provenance: &ResearchProvenanceFingerprint,
        build: Option<crate::research_worker::ResearchBuild>,
    ) -> StoreResult<()> {
        let build = build.ok_or(StoreError::AdmissionDenied)?;
        if provenance
            .source_code_sha256
            .as_ref()
            .map(|digest| digest.value.as_slice())
            != Some(build.source_sha256.as_slice())
            || provenance
                .environment_sha256
                .as_ref()
                .map(|digest| digest.value.as_slice())
                != Some(build.environment_sha256.as_slice())
        {
            return Err(StoreError::Corrupt(
                "worker build differs from research context",
            ));
        }
        Ok(())
    }

    fn resolve_result(
        &self,
        job: &JobSpecification,
        success: &JobSuccess,
    ) -> StoreResult<BacktestResult> {
        let frozen = self.frozen(job)?;
        let (wire, manifest, artifact) = frozen
            .result
            .as_ref()
            .ok_or(StoreError::Unavailable("unprepared result manifest"))?;
        verify_outputs(success, manifest, artifact)?;
        Ok(wire.clone())
    }

    fn resolve_current(
        &self,
        principal: &Actor,
        job: &JobSpecification,
        context: &str,
    ) -> StoreResult<Option<ResearchProvenanceFingerprint>> {
        self.authorize(principal)?;
        self.frozen(job)?;
        Ok(Some(
            self.current(principal, context)?.context.provenance.clone(),
        ))
    }

    fn resolve_perturbation_space(
        &self,
        principal: &Actor,
        job: &JobSpecification,
        context: &str,
    ) -> StoreResult<PerturbationSpace> {
        self.authorize(principal)?;
        let frozen = self.frozen(job)?;
        let current = self.current(principal, context)?;
        let family = current
            .family
            .as_ref()
            .ok_or(StoreError::Unavailable("unregistered perturbation family"))?;
        if family.provenance.as_ref() != Some(&frozen.context.provenance)
            || family
                .backtest_seed
                .as_ref()
                .map(|seed| seed.value.as_slice())
                != Some(model::parse_digest(&frozen.specification.deterministic_seed)?.as_slice())
            || !family.candidates.iter().any(|candidate| {
                candidate
                    .factor_spec_id
                    .as_ref()
                    .is_some_and(|id| id.value == factor_spec_id(&frozen.factor).to_string())
            })
        {
            return Err(StoreError::Corrupt("perturbation source family"));
        }
        Ok(family.clone())
    }

    fn resolve_admission(
        &self,
        principal: &Actor,
        job: &JobSpecification,
        context: &str,
    ) -> StoreResult<AdmissionEvidence> {
        self.authorize(principal)?;
        let frozen = self.frozen(job)?;
        let current = self.current(principal, context)?;
        if current.context.provenance != frozen.context.provenance
            || current.context.dataset.sample.role != model::SampleRole::InSample
        {
            return Err(StoreError::AdmissionDenied);
        }
        frozen
            .review
            .clone()
            .ok_or(StoreError::Unavailable("unregistered admission report"))
    }
}

fn verify_outputs(
    success: &JobSuccess,
    result: &model::ResultManifest,
    manifest: &ArtifactRef,
) -> StoreResult<()> {
    if success.outputs.len() != 9 || !success.outputs.contains(manifest) {
        return Err(StoreError::Corrupt("result manifest output binding"));
    }
    for artifact in result.artifacts.entries() {
        if !success.outputs.contains(&artifact.wire()?) {
            return Err(StoreError::Corrupt("result series output binding"));
        }
    }
    Ok(())
}

pub(super) fn check_files(files: &[Arc<VerifiedFile>]) -> StoreResult<()> {
    for file in files {
        file.check()?;
    }
    Ok(())
}
