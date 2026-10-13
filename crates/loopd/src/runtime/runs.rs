//! Frozen, human-owned research runs; supplier and numerical logic stay elsewhere.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use loop_protocol::wire::{runs::v1 as wire, v1};
use prost::Message;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{DiscoveryExecutor, deployment::read_file};
use crate::manifests::ObjectRef;
use crate::manifests::numerical::{FrozenNumerical, NumericalResolver};
use crate::store::{Clock, StoreError, StoreResult, SystemClock, validate_id};

/// Administrator-owned immutable run documents. Missing configuration disables
/// creation/advancement while durable owner-scoped status remains available.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    /// Private flat content-addressed configuration namespace, never research data.
    pub plan_store: PathBuf,
    /// One through 64 pinned run documents; callers can select only these plans.
    pub plans: Vec<ObjectRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    id: String,
    revision: String,
    specification: ObjectRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    numerical_recipe: Option<ObjectRef>,
}

struct FrozenRun {
    specification: wire::RunSpecification,
    files: Vec<(PathBuf, ObjectRef)>,
    numerical_recipe: Option<ObjectRef>,
}

/// Startup-pinned run plans sharing the existing Discovery executor. The catalog
/// grants no transport identity, budget reservation, lease or holdout capability.
pub struct RunCatalog {
    plans: Vec<FrozenRun>,
    discovery: Arc<DiscoveryExecutor>,
    numerical: Option<(PathBuf, Arc<NumericalResolver>)>,
}

impl RunCatalog {
    /// Resolve bounded private documents and canonical protobuf templates.
    /// A template must omit its own policy reference to avoid circular hashing;
    /// the enclosing document's digest becomes that immutable reference.
    /// Missing files, duplicate run IDs, unknown fields or executor drift deny
    /// startup. Numerical plans share a bounded file cache pinned to the trusted
    /// deployment root; each operation still resolves and verifies fresh proofs.
    /// Loading performs no network call or database mutation.
    pub fn open(
        config: RunConfig,
        discovery: Arc<DiscoveryExecutor>,
        artifact_root: &Path,
    ) -> StoreResult<Self> {
        if !(1..=64).contains(&config.plans.len()) {
            return Err(StoreError::Invalid("run catalog bounds"));
        }
        let mut plans: Vec<FrozenRun> = Vec::new();
        for reference in config.plans {
            let path = object_path(&config.plan_store, &reference)?;
            let document: Document = serde_json::from_slice(&checked_read(&path, &reference)?)
                .map_err(|_| StoreError::Invalid("run document JSON"))?;
            if !matches!(
                document.schema.as_str(),
                "loop.research-run/v1" | "loop.research-run/v2"
            ) {
                return Err(StoreError::Invalid("run document schema"));
            }
            validate_id(&document.id)?;
            validate_id(&document.revision)?;
            let specification_path = object_path(&config.plan_store, &document.specification)?;
            let bytes = checked_read(&specification_path, &document.specification)?;
            let mut specification = wire::RunSpecification::decode(bytes.as_slice())
                .map_err(|_| StoreError::Invalid("run specification protobuf"))?;
            if specification.plan.is_some() || specification.encode_to_vec() != bytes {
                return Err(StoreError::Invalid("run specification encoding"));
            }
            let numerical = document.schema == "loop.research-run/v2";
            if numerical != specification.numerical.is_some()
                || numerical != document.numerical_recipe.is_some()
            {
                return Err(StoreError::Invalid("run workflow version"));
            }
            let mut files = vec![
                (path, reference.clone()),
                (specification_path, document.specification),
            ];
            if let Some(recipe) = &document.numerical_recipe {
                let recipe_path = object_path(&config.plan_store, recipe)?;
                checked_read(&recipe_path, recipe)?;
                if specification
                    .numerical
                    .as_ref()
                    .and_then(|workflow| workflow.recipe.as_ref())
                    .and_then(|policy| policy.sha256.as_ref())
                    .map(|digest| digest.value.as_slice())
                    != Some(recipe.digest()?.as_slice())
                {
                    return Err(StoreError::Invalid("run recipe binding"));
                }
                files.push((recipe_path, recipe.clone()));
            }
            specification.plan = Some(v1::PolicyReference {
                policy_id: Some(v1::PolicyId { value: document.id }),
                revision: document.revision,
                sha256: Some(v1::Sha256Digest {
                    value: reference.digest()?.to_vec(),
                }),
            });
            let now = SystemClock.now_millis()?;
            loop_protocol::runs::validate_specification(
                &specification,
                &prost_types::Timestamp {
                    seconds: now / 1_000,
                    nanos: ((now % 1_000) * 1_000_000) as i32,
                },
            )?;
            discovery.verify_run(&specification)?;
            if plans.iter().any(|plan| {
                plan.specification.run_id == specification.run_id
                    || plan.specification.plan == specification.plan
            }) {
                return Err(StoreError::Invalid("duplicate run plan"));
            }
            plans.push(FrozenRun {
                specification,
                files,
                numerical_recipe: document.numerical_recipe,
            });
        }
        let numerical = plans
            .iter()
            .any(|plan| plan.numerical_recipe.is_some())
            .then(|| {
                NumericalResolver::open(&config.plan_store, artifact_root)
                    .map(|resolver| (artifact_root.to_owned(), Arc::new(resolver)))
            })
            .transpose()?;
        Ok(Self {
            plans,
            discovery,
            numerical,
        })
    }

    pub(super) fn specifications(&self) -> impl Iterator<Item = &wire::RunSpecification> {
        self.plans.iter().map(|plan| &plan.specification)
    }

    pub(super) fn contains(&self, reference: Option<&v1::PolicyReference>) -> bool {
        reference.is_some_and(|reference| {
            self.plans
                .iter()
                .any(|plan| plan.specification.plan.as_ref() == Some(reference))
        })
    }

    pub(super) fn resolve(
        &self,
        reference: &v1::PolicyReference,
    ) -> StoreResult<&wire::RunSpecification> {
        let plan = self
            .plans
            .iter()
            .find(|plan| plan.specification.plan.as_ref() == Some(reference))
            .ok_or(StoreError::AdmissionDenied)?;
        for (path, reference) in &plan.files {
            checked_read(path, reference)?;
        }
        self.discovery.verify_run(&plan.specification)?;
        Ok(&plan.specification)
    }

    pub(super) fn verify(&self, specification: &wire::RunSpecification) -> StoreResult<()> {
        let reference = specification
            .plan
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        if self.resolve(reference)? != specification {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }

    pub(super) async fn numerical(
        &self,
        specification: &wire::RunSpecification,
        artifact_root: &Path,
    ) -> StoreResult<(Arc<NumericalResolver>, Arc<FrozenNumerical>)> {
        self.verify(specification)?;
        let (root, resolver) = self.numerical.as_ref().ok_or(StoreError::AdmissionDenied)?;
        if root != artifact_root {
            return Err(StoreError::AdmissionDenied);
        }
        let plan = self
            .plans
            .iter()
            .find(|plan| plan.specification == *specification)
            .ok_or(StoreError::AdmissionDenied)?;
        let workflow = specification
            .numerical
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let dataset = specification
            .discovery
            .as_ref()
            .and_then(|input| input.dataset.as_ref())
            .ok_or(StoreError::AdmissionDenied)?;
        let frozen = resolver
            .prepare(
                workflow
                    .recipe
                    .as_ref()
                    .ok_or(StoreError::AdmissionDenied)?,
                plan.numerical_recipe
                    .as_ref()
                    .ok_or(StoreError::AdmissionDenied)?,
                dataset,
            )
            .await?;
        Ok((resolver.clone(), Arc::new(frozen)))
    }
}

fn object_path(root: &Path, reference: &ObjectRef) -> StoreResult<PathBuf> {
    reference.digest()?;
    Ok(root.join(&reference.sha256[7..]))
}

fn checked_read(path: &Path, reference: &ObjectRef) -> StoreResult<Vec<u8>> {
    if !(1..=1_048_576).contains(&reference.byte_size) {
        return Err(StoreError::Invalid("run object size"));
    }
    let bytes = read_file(path, true, reference.byte_size)?;
    if bytes.len() as u64 != reference.byte_size
        || Sha256::digest(&bytes).as_slice() != reference.digest()?
    {
        return Err(StoreError::Corrupt("run object content"));
    }
    Ok(bytes)
}
