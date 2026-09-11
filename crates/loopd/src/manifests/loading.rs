use std::collections::BTreeSet;
use std::sync::Arc;

use loop_core::factor::{
    FactorSpec, FactorSpecId, OperatorPolicyRegistry, PolicyRef, ValidationLimits,
    parse_canonical_factor_spec,
};
use loop_protocol::wire::v1::{BacktestId, BacktestResult, ResearchProvenanceFingerprint};
use serde::{Serialize, de::DeserializeOwned};

use super::files::{ReadBudget, VerifiedFile};
use super::{LocalArtifacts, ObjectRef, model};
use crate::store::{StoreError, StoreResult};

pub(super) struct Materializer<'a> {
    pub store: &'a LocalArtifacts,
    pub registries: &'a [Arc<OperatorPolicyRegistry>],
    pub files: Vec<Arc<VerifiedFile>>,
    budget: ReadBudget,
}

pub(super) struct ResolvedContext {
    pub manifest: model::Context,
    pub registry: Arc<OperatorPolicyRegistry>,
    pub provenance: ResearchProvenanceFingerprint,
    pub dataset: model::Dataset,
    pub policies: Vec<model::Policy>,
    pub policy_documents: Vec<model::PolicyDocument>,
    pub engine: model::Engine,
    pub engine_version: String,
}

impl<'a> Materializer<'a> {
    pub fn new(store: &'a LocalArtifacts, registries: &'a [Arc<OperatorPolicyRegistry>]) -> Self {
        Self {
            store,
            registries,
            files: Vec::new(),
            budget: ReadBudget::new(),
        }
    }

    pub async fn object(
        &mut self,
        reference: &ObjectRef,
        metadata: bool,
    ) -> StoreResult<Arc<VerifiedFile>> {
        let file = self
            .store
            .load(reference, metadata, &mut self.budget)
            .await?;
        self.files.push(Arc::clone(&file));
        Ok(file)
    }

    pub async fn json<T: DeserializeOwned + Serialize>(
        &mut self,
        reference: &ObjectRef,
    ) -> StoreResult<T> {
        self.object(reference, true).await?.json()
    }

    pub async fn context(&mut self, reference: &ObjectRef) -> StoreResult<ResolvedContext> {
        let context: model::Context = self.json(reference).await?;
        model::schema(&context.schema, "loop.research-context/v1")?;
        self.file_set(&context.source, "loop.source-files/v1")
            .await?;
        self.file_set(&context.environment, "loop.environment-files/v1")
            .await?;
        let registry_file = self.object(&context.registry, true).await?;
        let registry_bytes = registry_file.bytes()?;
        let registry = self
            .registries
            .iter()
            .find(|registry| registry.canonical_bytes().as_slice() == registry_bytes)
            .cloned()
            .ok_or(StoreError::AdmissionDenied)?;
        let configuration: model::Configuration = self.json(&context.configuration).await?;
        model::schema(&configuration.schema, "loop.research-configuration/v1")?;
        model::text(&configuration.backtest_engine_version)?;
        sorted(
            configuration
                .policies
                .iter()
                .map(|policy| policy.policy_id.as_str()),
            1,
            32,
        )?;
        let mut policy_documents = Vec::new();
        for policy in &configuration.policies {
            policy.validate()?;
            let document: model::PolicyDocument = self.json(&policy.document).await?;
            model::schema(&document.schema, "loop.research-policy/v1")?;
            if document.policy_id != policy.policy_id
                || document.revision != policy.revision
                || document.settings.len() > 128
            {
                return Err(StoreError::Corrupt("policy document binding"));
            }
            for (name, value) in &document.settings {
                model::text(name)?;
                model::text(value)?;
            }
            policy_documents.push(document);
        }
        let data: model::Dataset = self.json(&context.data).await?;
        model::schema(&data.schema, "loop.development-dataset/v1")?;
        data.sample.validate()?;
        sorted(
            data.snapshots
                .iter()
                .map(|snapshot| snapshot.snapshot_id.as_str()),
            1,
            128,
        )?;
        for snapshot in &data.snapshots {
            model::text(&snapshot.source)?;
            model::text(&snapshot.dataset)?;
            model::text(&snapshot.entitlement)?;
            model::timestamp(snapshot.known_through_ms)?;
            let end = model::date(&data.sample.end)?
                .and_hms_opt(23, 59, 59)
                .ok_or(StoreError::Corrupt("sample end"))?
                .and_utc()
                .timestamp_millis()
                + 999;
            if snapshot.known_through_ms > end
                || snapshot.artifacts.is_empty()
                || snapshot.artifacts.len() > 256
            {
                return Err(StoreError::Corrupt("snapshot information boundary"));
            }
            let mut identities = BTreeSet::new();
            for artifact in &snapshot.artifacts {
                if !identities.insert(&artifact.object.sha256) {
                    return Err(StoreError::Corrupt("duplicate data artifact"));
                }
                self.artifact(artifact).await?;
            }
        }
        let calendar: model::Calendar = self.json(&context.calendar).await?;
        model::schema(&calendar.schema, "loop.trading-calendar/v1")?;
        if calendar.name != "XNYS" || calendar.timezone != "America/New_York" {
            return Err(StoreError::AdmissionDenied);
        }
        sorted(calendar.sessions.iter().map(String::as_str), 1, 10_000)?;
        let mut in_window = false;
        for session in &calendar.sessions {
            model::date(session)?;
            in_window |= session >= &data.sample.start && session <= &data.sample.end;
        }
        if !in_window {
            return Err(StoreError::Corrupt("calendar has no sample sessions"));
        }
        Ok(ResolvedContext {
            provenance: context.provenance(*registry.identity().as_bytes())?,
            manifest: context,
            registry,
            dataset: data,
            policies: configuration.policies,
            policy_documents,
            engine: configuration.backtest_engine,
            engine_version: configuration.backtest_engine_version,
        })
    }

    async fn file_set(
        &mut self,
        reference: &ObjectRef,
        schema: &str,
    ) -> StoreResult<Vec<model::NamedFile>> {
        let set: model::FileSet = self.json(reference).await?;
        model::schema(&set.schema, schema)?;
        sorted(set.files.iter().map(|file| file.name.as_str()), 1, 8192)?;
        for file in &set.files {
            if file.name.starts_with('/')
                || file.name.contains('\\')
                || file
                    .name
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(StoreError::Corrupt("build file name"));
            }
            self.object(&file.object, false).await?;
        }
        Ok(set.files)
    }

    pub async fn artifact(&mut self, artifact: &model::Artifact) -> StoreResult<()> {
        artifact.wire()?;
        let schema: model::SchemaDocument = self.json(&artifact.schema.document).await?;
        model::schema(&schema.schema, "loop.artifact-schema/v1")?;
        if schema.name != artifact.schema.name
            || schema.version != artifact.schema.version
            || schema.media_type != artifact.media_type
            || schema.columns.len() > 256
            || schema.columns.iter().collect::<BTreeSet<_>>().len() != schema.columns.len()
        {
            return Err(StoreError::Corrupt("artifact schema binding"));
        }
        for column in schema.columns {
            model::text(&column)?;
        }
        self.object(&artifact.object, false).await?;
        Ok(())
    }

    pub async fn factor(
        &mut self,
        reference: &ObjectRef,
        context: &ResolvedContext,
    ) -> StoreResult<FactorSpec> {
        let factor: model::Factor = self.json(reference).await?;
        model::schema(&factor.schema, "loop.factor-manifest/v1")?;
        let id = FactorSpecId::parse(&factor.factor_spec_id)
            .map_err(|_| StoreError::Corrupt("factor identity"))?;
        let specification = self.object(&factor.specification, true).await?;
        let expression = self.object(&factor.expression, true).await?;
        let factor = parse_canonical_factor_spec(
            specification.bytes()?,
            id,
            expression.bytes()?,
            &context.registry,
            ValidationLimits::default(),
        )
        .map_err(|_| StoreError::Corrupt("canonical factor binding"))?;
        for policy in policies(&factor) {
            if !context.policies.iter().any(|resolved| {
                resolved.policy_id == policy.policy_id().as_str()
                    && resolved.revision == policy.revision().as_str()
                    && resolved
                        .document
                        .digest()
                        .is_ok_and(|hash| hash == *policy.sha256())
            }) {
                return Err(StoreError::Corrupt("frozen factor policy"));
            }
        }
        Ok(factor)
    }

    pub async fn result(
        &mut self,
        artifact: &model::Artifact,
        entry: &model::BacktestEntry,
        specification: &model::Backtest,
        context: &ResolvedContext,
    ) -> StoreResult<(BacktestResult, model::ResultManifest)> {
        self.artifact(artifact).await?;
        if artifact.schema.name != "loop.backtest_result"
            || artifact.schema.version != 1
            || artifact.media_type != "application/json"
        {
            return Err(StoreError::Corrupt("result artifact type"));
        }
        let result: model::ResultManifest = self.json(&artifact.object).await?;
        model::schema(&result.schema, "loop.backtest-result/v1")?;
        if result.job_id != entry.job_id
            || result.specification != entry.specification
            || artifact.created_at_ms != result.completed_at_ms
            || result.engine != specification.engine
            || result.engine_version != specification.engine_version
        {
            return Err(StoreError::Corrupt("result frozen specification"));
        }
        model::text(&result.engine_version)?;
        sorted(
            result.metrics.iter().map(|metric| metric.name.as_str()),
            1,
            256,
        )?;
        for artifact in result.artifacts.entries() {
            if artifact.created_at_ms > result.completed_at_ms {
                return Err(StoreError::Corrupt("result artifact time"));
            }
            self.artifact(artifact).await?;
        }
        let wire = BacktestResult {
            backtest_id: Some(BacktestId {
                value: specification.backtest_id.clone(),
            }),
            engine: result.engine.wire(),
            engine_version: result.engine_version.clone(),
            provenance: Some(context.provenance.clone()),
            metrics: result
                .metrics
                .iter()
                .map(model::Metric::wire)
                .collect::<StoreResult<_>>()?,
            artifacts: Some(result.artifacts.wire()?),
            result_manifest_sha256: Some(model::digest(artifact.object.digest()?)),
            completed_at: Some(model::timestamp(result.completed_at_ms)?),
        };
        Ok((wire, result))
    }
}

pub(super) fn policies(factor: &FactorSpec) -> [&PolicyRef; 9] {
    [
        factor.universe_policy(),
        factor.data_policy(),
        factor.calendar_policy(),
        factor.preprocess_policy(),
        factor.neutralization_policy(),
        factor.portfolio_policy(),
        factor.execution_policy(),
        factor.cost_policy(),
        factor.evaluation_policy(),
    ]
}

pub(super) fn sorted<'a>(
    items: impl Iterator<Item = &'a str>,
    minimum: usize,
    maximum: usize,
) -> StoreResult<()> {
    let mut previous = None;
    let mut count = 0;
    for item in items {
        model::text(item)?;
        if previous.is_some_and(|old| old >= item) {
            return Err(StoreError::Corrupt("manifest ordering or duplicate"));
        }
        previous = Some(item);
        count += 1;
        if count > maximum {
            return Err(StoreError::Invalid("manifest item limit"));
        }
    }
    if count < minimum {
        return Err(StoreError::Corrupt("empty manifest collection"));
    }
    Ok(())
}
