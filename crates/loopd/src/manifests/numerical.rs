//! File-backed derivation of numerical children from a frozen research recipe.
//! These proofs never grant a transport identity, job lease or reservation.

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loop_core::factor::{
    self, FactorSpecDraft, PolicyId, PolicyRef, PositiveInteger, ValidationLimits,
};
use loop_protocol::wire::v1 as wire;
use serde::{Deserialize, Serialize};

use super::files::{ReadBudget, VerifiedFile};
use super::loading::{Materializer, ResolvedContext};
use super::policy::check_files;
use super::{EvaluationPin, EvaluationResolver, LocalArtifacts, ObjectRef, PortfolioPin, model};
use crate::store::{StoreError, StoreResult};

mod family;
mod portfolio;
mod publication;
pub(crate) use family::VerifiedFamily;

const POLICY_ROLES: [&str; 9] = [
    "universe_policy",
    "data_policy",
    "calendar_policy",
    "preprocess_policy",
    "neutralization_policy",
    "portfolio_policy",
    "execution_policy",
    "cost_policy",
    "evaluation_policy",
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema: String,
    id: String,
    revision: String,
    evaluation_context: ObjectRef,
    direction: factor::FactorDirection,
    deterministic_seed: String,
    execution_tape: ObjectRef,
    policies: BTreeMap<String, String>,
    hypothesis: String,
    required_observations: Vec<String>,
    falsification_criteria: Vec<String>,
}

/// Resolver for the explicit recipe and its actual numerical dependencies.
/// Publication is append-only; replay only verifies already published objects.
pub(crate) struct NumericalResolver {
    pub(super) plans: LocalArtifacts,
    pub(super) artifacts: LocalArtifacts,
    evaluation: EvaluationResolver,
    root: PathBuf,
    identity: (u64, u64),
    registry: Arc<factor::OperatorPolicyRegistry>,
}

/// Immutable administrator declaration and verified numerical context.
/// Narrative fields are recipe-declared claims, never model or result evidence.
pub(crate) struct FrozenNumerical {
    reference: ObjectRef,
    recipe: Recipe,
    context: ResolvedContext,
    files: Vec<Arc<VerifiedFile>>,
}

/// Inputs from the actual successful evaluation and its registered result.
/// The caller must prove that `result` is the transactionally registered trial.
pub(crate) struct PortfolioDerivation<'a> {
    pub record: &'a wire::JobRecord,
    pub result: &'a wire::FactorEvaluationResult,
    pub budget: wire::JobBudget,
    pub job_id: &'a str,
    pub backtest_id: &'a str,
    pub outputs: &'a LocalArtifacts,
    pub replay: bool,
}

/// Complete derived references, not submission authority. All referenced source
/// and publication guards are retained until the parent transition commits.
pub(crate) struct DerivedPortfolio {
    pub input: wire::BacktestJobInput,
    pub pin: PortfolioPin,
    pub context: ObjectRef,
    files: Vec<Arc<VerifiedFile>>,
}

impl NumericalResolver {
    pub(crate) fn open(plan_root: &Path, artifact_root: &Path) -> StoreResult<Self> {
        if !artifact_root.is_absolute()
            || std::fs::canonicalize(artifact_root).ok().as_deref() != Some(artifact_root)
        {
            return Err(StoreError::Invalid("numerical publication root"));
        }
        let metadata = std::fs::symlink_metadata(artifact_root)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o022 != 0
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(StoreError::Invalid("numerical publication ownership"));
        }
        Ok(Self {
            plans: LocalArtifacts::open(plan_root)?,
            artifacts: LocalArtifacts::open(artifact_root)?,
            evaluation: EvaluationResolver::open(artifact_root, vec![])?,
            root: artifact_root.to_owned(),
            identity: (metadata.dev(), metadata.ino()),
            registry: Arc::new(
                factor::us_equities::registry()
                    .map_err(|_| StoreError::Invalid("installed numerical registry"))?,
            ),
        })
    }

    pub(crate) async fn prepare(
        &self,
        reference: &wire::PolicyReference,
        object: &ObjectRef,
        dataset: &wire::DevelopmentDatasetReference,
    ) -> StoreResult<FrozenNumerical> {
        self.check_root()?;
        if reference
            .sha256
            .as_ref()
            .map(|digest| digest.value.as_slice())
            != Some(object.digest()?.as_slice())
        {
            return Err(StoreError::Corrupt("numerical recipe digest"));
        }
        let file = self
            .plans
            .load(object, true, &mut ReadBudget::new())
            .await?;
        let recipe: Recipe = file.json()?;
        model::schema(&recipe.schema, "loop.run-numerical/v1")?;
        PolicyId::new(&recipe.id).map_err(|_| StoreError::Invalid("numerical recipe ID"))?;
        PositiveInteger::new(&recipe.revision)
            .map_err(|_| StoreError::Invalid("numerical recipe revision"))?;
        if reference.policy_id.as_ref().map(|id| id.value.as_str()) != Some(recipe.id.as_str())
            || reference.revision != recipe.revision
        {
            return Err(StoreError::Corrupt("numerical recipe identity"));
        }
        model::parse_digest(&recipe.deterministic_seed)?;
        narrative(&recipe.hypothesis)?;
        for values in [
            &recipe.required_observations,
            &recipe.falsification_criteria,
        ] {
            if !(1..=16).contains(&values.len()) {
                return Err(StoreError::Invalid("numerical narrative bounds"));
            }
            for value in values {
                narrative(value)?;
            }
        }
        let registries = [self.registry.clone()];
        let mut loader = Materializer::for_evaluation(&self.artifacts, &registries);
        let context = loader.context(&recipe.evaluation_context).await?;
        if context.dataset.sample.role != model::SampleRole::InSample
            || &context.dataset.reference(&context.manifest.data)? != dataset
            || context.engine != model::Engine::PrimaryCrossSectional
            || !matches!(
                context.engine_version.as_str(),
                "factor-evaluator.2" | "factor-evaluator.3"
            )
            || context.portfolio_request.is_some()
        {
            return Err(StoreError::AdmissionDenied);
        }
        let mut policy_ids = BTreeSet::new();
        if recipe.policies.len() != POLICY_ROLES.len() {
            return Err(StoreError::Invalid("numerical policy roles"));
        }
        for role in POLICY_ROLES {
            let id = recipe
                .policies
                .get(role)
                .ok_or(StoreError::Invalid("numerical policy role"))?;
            if !policy_ids.insert(id)
                || !context
                    .policies
                    .iter()
                    .any(|policy| &policy.policy_id == id)
            {
                return Err(StoreError::Corrupt("numerical policy binding"));
            }
        }
        if !context
            .dataset
            .snapshots
            .iter()
            .flat_map(|snapshot| &snapshot.artifacts)
            .any(|artifact| {
                artifact.object == recipe.execution_tape
                    && artifact.schema.name == "loop.execution_tape"
                    && artifact.schema.version == 1
                    && artifact.media_type == "application/json"
            })
        {
            return Err(StoreError::Corrupt("numerical execution tape binding"));
        }
        loader.files.push(file);
        check_files(&loader.files)?;
        Ok(FrozenNumerical {
            reference: object.clone(),
            recipe,
            context,
            files: loader.files,
        })
    }
}

impl FrozenNumerical {
    pub(crate) fn check(&self) -> StoreResult<()> {
        check_files(&self.files)
    }

    pub(crate) fn recipe(&self) -> &ObjectRef {
        &self.reference
    }

    pub(crate) fn data_manifest(&self) -> &ObjectRef {
        &self.context.manifest.data
    }

    pub(crate) fn evaluation_pin(&self, job_id: &str) -> EvaluationPin {
        EvaluationPin {
            job_id: job_id.to_owned(),
            context: self.recipe.evaluation_context.clone(),
        }
    }

    pub(crate) fn evaluation(
        &self,
        ast: &wire::CanonicalFactorAst,
        budget: wire::JobBudget,
    ) -> StoreResult<wire::FactorEvaluationJobInput> {
        self.check()?;
        let expression = ast
            .expression_id
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let domain_policy = |role: &str| -> StoreResult<PolicyRef> {
            let id = self
                .recipe
                .policies
                .get(role)
                .ok_or(StoreError::Corrupt("numerical policy role"))?;
            let policy = self
                .context
                .policies
                .iter()
                .find(|policy| &policy.policy_id == id)
                .ok_or(StoreError::Corrupt("numerical policy reference"))?;
            Ok(PolicyRef::new(
                PolicyId::new(id).map_err(|_| StoreError::Invalid("policy ID"))?,
                PositiveInteger::new(&policy.revision)
                    .map_err(|_| StoreError::Invalid("policy revision"))?,
                policy.document.digest()?,
            ))
        };
        let draft = FactorSpecDraft::new(
            factor::ExpressionId::parse(&expression.value)
                .map_err(|_| StoreError::Invalid("numerical expression"))?,
            *self.context.registry.identity().as_bytes(),
            self.recipe.direction,
            domain_policy("universe_policy")?,
            domain_policy("data_policy")?,
            domain_policy("calendar_policy")?,
            domain_policy("preprocess_policy")?,
            domain_policy("neutralization_policy")?,
            domain_policy("portfolio_policy")?,
            domain_policy("execution_policy")?,
            domain_policy("cost_policy")?,
            domain_policy("evaluation_policy")?,
        );
        let bound = factor::bind_factor_spec(
            draft,
            &ast.canonical_json,
            &self.context.registry,
            ValidationLimits::default(),
        )
        .map_err(|_| StoreError::Invalid("canonical numerical factor"))?;
        let wire_policy = |role: &str| -> StoreResult<Option<wire::PolicyReference>> {
            let policy = domain_policy(role)?;
            Ok(Some(wire::PolicyReference {
                policy_id: Some(wire::PolicyId {
                    value: policy.policy_id().as_str().to_owned(),
                }),
                revision: policy.revision().as_str().to_owned(),
                sha256: Some(model::digest(*policy.sha256())),
            }))
        };
        let factor = wire::FactorSpec {
            factor_spec_id: Some(wire::FactorSpecId {
                value: factor::factor_spec_id(&bound).to_string(),
            }),
            expression_id: Some(expression.clone()),
            expression: Some(ast.clone()),
            direction: match self.recipe.direction {
                factor::FactorDirection::HigherIsBetter => wire::FactorDirection::HigherIsBetter,
                factor::FactorDirection::LowerIsBetter => wire::FactorDirection::LowerIsBetter,
            } as i32,
            frozen_policy: Some(wire::FrozenResearchPolicyReference {
                universe_policy: wire_policy("universe_policy")?,
                data_policy: wire_policy("data_policy")?,
                calendar_policy: wire_policy("calendar_policy")?,
                preprocess_policy: wire_policy("preprocess_policy")?,
                neutralization_policy: wire_policy("neutralization_policy")?,
                portfolio_policy: wire_policy("portfolio_policy")?,
                execution_policy: wire_policy("execution_policy")?,
                cost_policy: wire_policy("cost_policy")?,
                evaluation_policy: wire_policy("evaluation_policy")?,
            }),
            operator_registry_sha256: Some(model::digest(
                *self.context.registry.identity().as_bytes(),
            )),
        };
        loop_protocol::job::validate_factor_identity(&factor)?;
        if loop_protocol::job::factor_identity_bytes(&factor)? != factor::factor_spec_bytes(&bound)
        {
            return Err(StoreError::Corrupt("numerical factor encoding"));
        }
        Ok(wire::FactorEvaluationJobInput {
            factor: Some(factor),
            dataset: Some(
                self.context
                    .dataset
                    .reference(&self.context.manifest.data)?,
            ),
            budget: Some(budget),
            provenance: Some(self.context.provenance.clone()),
            deterministic_seed: Some(model::digest(model::parse_digest(
                &self.recipe.deterministic_seed,
            )?)),
        })
    }
}

impl DerivedPortfolio {
    pub(crate) fn check(&self) -> StoreResult<()> {
        check_files(&self.files)
    }
}

fn narrative(value: &str) -> StoreResult<()> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > 2048
        || value.chars().any(char::is_control)
    {
        return Err(StoreError::Invalid("numerical narrative text"));
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests;
