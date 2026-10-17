//! Immutable inputs and private completion evidence for the installed worker.
//! Database registration, authority, lease acquisition and budgeting stay in
//! the store/runtime. Creating these objects does not authorize execution.

use std::collections::BTreeSet;
use std::sync::Arc;

use loop_protocol::wire::v1 as wire;
use prost::Message;
use serde::{Deserialize, Serialize};

use super::feedback::{FrozenFeedback, VerifiedObservation};
use super::files::{ReadBudget, VerifiedFile};
use super::numerical::{FrozenNumerical, NumericalResolver, VerifiedFamily};
use super::policy::check_files;
use super::{ObjectRef, model};
use crate::research_worker::ResearchBuild;
use crate::store::{StoreError, StoreResult};

mod receipt;
pub(crate) use receipt::{PerturbationCompletion, PerturbationEvidence};

#[cfg(test)]
mod tests;

/// Server-derived arguments from one exact parent transition. Failure memory
/// and before-state still require the parent's persisted binding validation.
pub(crate) struct PerturbationRequest {
    pub policy: Arc<FrozenFeedback>,
    pub family: Arc<VerifiedFamily>,
    pub work: wire::PerturbationWork,
    pub sources: Vec<Arc<VerifiedObservation>>,
    pub parent_binding: [u8; 32],
    pub budget: wire::JobBudget,
    pub created_at_ms: i64,
    pub replay: bool,
}

/// Verified actual work bytes and source guards; fields cannot be forged by a
/// caller supplying a JobSuccess or an arbitrary worker state.
pub(crate) struct PreparedPerturbation {
    input: wire::PerturbationJobInput,
    work: wire::PerturbationWork,
    reference: ObjectRef,
    family: Arc<VerifiedFamily>,
    policy: Arc<FrozenFeedback>,
    observations: Vec<Arc<VerifiedObservation>>,
    sources: Vec<(wire::JobRecord, wire::BacktestResult)>,
    build: ResearchBuild,
    files: Vec<Arc<VerifiedFile>>,
}

impl NumericalResolver {
    /// Read the exact immutable input of an earlier perturbation without
    /// recreating it. The returned message is not an execution proof: callers
    /// must prepare it against the frozen family/sources and recover its receipt
    /// before consuming historical state or committing a parent transition.
    pub(crate) async fn read_work(
        &self,
        record: &wire::JobRecord,
    ) -> StoreResult<wire::PerturbationWork> {
        self.check_root()?;
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("perturbation input job"))?;
        loop_protocol::job::validate_job_specification(job)?;
        let Some(wire::job_specification::Input::Perturbation(input)) = &job.input else {
            return Err(StoreError::Corrupt("perturbation input kind"));
        };
        let reference = input
            .work
            .as_ref()
            .ok_or(StoreError::Corrupt("perturbation input work"))?;
        let object = receipt::object(
            reference,
            "loop.perturbation_work",
            "application/x-protobuf",
        )?;
        let file = self
            .artifacts
            .load(&object, true, &mut ReadBudget::new())
            .await?;
        let work = receipt::decode(file.bytes()?)?;
        let mut files = vec![file];
        let created_at_ms = crate::store::timestamp_millis(
            reference
                .created_at
                .as_ref()
                .ok_or(StoreError::Corrupt("perturbation work time"))?,
            false,
        )?;
        if artifact(
            self,
            object,
            "loop.perturbation_work",
            "application/x-protobuf",
            created_at_ms,
            true,
            &mut files,
        )
        .await?
            != *reference
        {
            return Err(StoreError::Corrupt("perturbation work envelope"));
        }
        self.check_root()?;
        check_files(&files)?;
        Ok(work)
    }

    /// Publish before acquiring a child lease. Replay only reads existing bytes.
    pub(crate) async fn perturbation(
        &self,
        frozen: &FrozenNumerical,
        request: PerturbationRequest,
    ) -> StoreResult<PreparedPerturbation> {
        frozen.check()?;
        request.policy.check()?;
        request.family.check()?;
        let build = frozen.build()?;
        let space = request.family.space();
        let before = request
            .work
            .state
            .as_ref()
            .ok_or(StoreError::Corrupt("perturbation before-state"))?;
        crate::store::validate_perturbation_state(before, space)?;
        if request.sources.len() > 64
            || !request.family.binds(frozen, &request.policy)
            || request.work.candidates != space.candidates
            || request.work.failed_factor_ids.len() > 64
            || !space
                .candidates
                .iter()
                .any(|candidate| candidate.window == request.work.current_window)
            || before.history.len() > 64
            || space
                .provenance
                .as_ref()
                .and_then(|value| value.source_code_sha256.as_ref())
                .map(|hash| hash.value.as_slice())
                != Some(build.source_sha256.as_slice())
            || space
                .provenance
                .as_ref()
                .and_then(|value| value.environment_sha256.as_ref())
                .map(|hash| hash.value.as_slice())
                != Some(build.environment_sha256.as_slice())
        {
            return Err(StoreError::Corrupt("perturbation frozen work"));
        }
        let mut failed = BTreeSet::new();
        for id in &request.work.failed_factor_ids {
            if request.family.candidate(id).is_none() || !failed.insert(&id.value) {
                return Err(StoreError::Corrupt("perturbation failure memory"));
            }
        }
        let mut jobs = BTreeSet::new();
        let mut eligible = Vec::new();
        let mut sources = Vec::new();
        let mut observed = BTreeSet::new();
        let mut previous_round = 0;
        for source in &request.sources {
            source.check()?;
            if !jobs.insert(source.entry().job_id.clone()) || source.entry().round <= previous_round
            {
                return Err(StoreError::Corrupt("perturbation duplicate source"));
            }
            previous_round = source.entry().round;
            observed.insert(source.entry().factor_spec_id.as_str());
            if let Some(value) = source.eligible(&request.family)? {
                eligible.push(value);
            }
            let (record, result) = source.source();
            sources.push((record.clone(), result.clone()));
        }
        let expected_observed: Vec<_> = space
            .candidates
            .iter()
            .filter_map(|candidate| candidate.factor_spec_id.as_ref())
            .filter(|id| observed.contains(id.value.as_str()))
            .cloned()
            .collect();
        if request.work.observed_factor_ids != expected_observed {
            return Err(StoreError::Corrupt("perturbation observed candidates"));
        }
        for observation in before.history.iter().chain(request.work.observation.iter()) {
            if !eligible.contains(observation) {
                return Err(StoreError::Corrupt("unverified Sharpe observation"));
            }
        }
        let latest = eligible.iter().rev().find(|source| {
            !before
                .history
                .iter()
                .any(|known| known.source_job_id == source.source_job_id)
        });
        if request.work.observation.as_ref() != latest {
            return Err(StoreError::Corrupt("perturbation latest source"));
        }
        if request.work.observation.is_none() && before.history.is_empty() {
            return Err(StoreError::Invalid(
                "perturbation requires an eligible source",
            ));
        }
        let mut files = frozen.guards();
        files.extend(request.policy.guards());
        files.extend(request.family.guards());
        let bytes = request.work.encode_to_vec();
        let reference = self.publish(&bytes, request.replay, &mut files).await?;
        let artifact = artifact(
            self,
            reference.clone(),
            "loop.perturbation_work",
            "application/x-protobuf",
            request.created_at_ms,
            request.replay,
            &mut files,
        )
        .await?;
        let policy = request.policy.policy();
        let input = wire::PerturbationJobInput {
            dataset: space.dataset.clone(),
            feedback_policy: Some(wire::PolicyReference {
                policy_id: Some(wire::PolicyId {
                    value: policy.id.clone(),
                }),
                revision: policy.revision.clone(),
                sha256: Some(model::digest(request.policy.reference().digest()?)),
            }),
            parent_binding_sha256: Some(model::digest(request.parent_binding)),
            family_sha256: Some(model::digest(request.family.digest())),
            work: Some(artifact),
            source_code_sha256: Some(model::digest(build.source_sha256)),
            environment_sha256: Some(model::digest(build.environment_sha256)),
            budget: Some(request.budget),
            algorithm: space.algorithm.clone(),
        };
        let prepared = PreparedPerturbation {
            input,
            work: request.work,
            reference,
            family: request.family,
            policy: request.policy,
            observations: request.sources,
            sources,
            build,
            files,
        };
        prepared.check()?;
        Ok(prepared)
    }
}

impl PreparedPerturbation {
    pub(crate) fn check(&self) -> StoreResult<()> {
        self.policy.check()?;
        self.family.check()?;
        for observation in &self.observations {
            observation.check()?;
        }
        check_files(&self.files)
    }

    pub(crate) fn input(&self) -> &wire::PerturbationJobInput {
        &self.input
    }
    pub(crate) fn work(&self) -> &wire::PerturbationWork {
        &self.work
    }
    pub(crate) fn build(&self) -> ResearchBuild {
        self.build
    }
    pub(crate) fn sources(&self) -> &[(wire::JobRecord, wire::BacktestResult)] {
        &self.sources
    }

    /// Pure metadata/guard validation suitable for the final submission transaction.
    pub(crate) fn bind_job(&self, job: &wire::JobSpecification) -> StoreResult<()> {
        self.check()?;
        if job.kind != wire::JobKind::Perturbation as i32
            || job.input
                != Some(wire::job_specification::Input::Perturbation(
                    self.input.clone(),
                ))
            || self.sources.iter().any(|(record, _)| {
                record
                    .specification
                    .as_ref()
                    .is_none_or(|source| source.run_id != job.run_id)
            })
        {
            return Err(StoreError::Corrupt("perturbation job binding"));
        }
        loop_protocol::job::validate_job_specification(job)?;
        Ok(())
    }
}

async fn artifact(
    resolver: &NumericalResolver,
    object: ObjectRef,
    name: &str,
    media_type: &str,
    created_at_ms: i64,
    replay: bool,
    files: &mut Vec<Arc<VerifiedFile>>,
) -> StoreResult<wire::ArtifactRef> {
    let schema = resolver
        .json(
            &model::SchemaDocument {
                schema: "loop.artifact-schema/v1".to_owned(),
                name: name.to_owned(),
                version: 1,
                media_type: media_type.to_owned(),
                columns: vec![],
            },
            replay,
            files,
        )
        .await?;
    model::Artifact {
        object,
        schema: model::SchemaRef {
            name: name.to_owned(),
            version: 1,
            document: schema,
        },
        media_type: media_type.to_owned(),
        created_at_ms,
    }
    .wire()
}
