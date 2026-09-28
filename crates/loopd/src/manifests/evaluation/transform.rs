//! Identity checks for Python-owned cross-sectional transformations.

use loop_core::factor::{FactorSpec, PolicyRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifests::{ObjectRef, model};
use crate::store::{StoreError, StoreResult};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PanelTransform {
    pub preprocess: model::PolicyDocument,
    pub neutralization: model::PolicyDocument,
    pub exposures: Option<ObjectRef>,
}

pub(super) struct BoundTransform {
    preprocess: String,
    neutralization: String,
    pub exposures: Option<ObjectRef>,
    sessions: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransformEvidence {
    profile: String,
    preprocess_sha256: String,
    neutralization_sha256: String,
    exposures_sha256: Option<String>,
    raw_valid_observations: u64,
    outcomes: Vec<String>,
}

impl PanelTransform {
    pub fn bind(&self, factor: &FactorSpec, sessions: usize) -> StoreResult<BoundTransform> {
        if sessions == 0 || sessions > 8192 {
            return Err(StoreError::Invalid("transformation session bounds"));
        }
        if let Some(reference) = &self.exposures {
            reference.digest()?;
            if reference.byte_size == 0 || reference.byte_size > 64 * 1024 * 1024 {
                return Err(StoreError::Invalid("exposure artifact bounds"));
            }
        }
        Ok(BoundTransform {
            preprocess: bind_policy(&self.preprocess, factor.preprocess_policy())?,
            neutralization: bind_policy(&self.neutralization, factor.neutralization_policy())?,
            exposures: self.exposures.clone(),
            sessions,
        })
    }
}

impl BoundTransform {
    pub fn check(
        &self,
        evidence: &TransformEvidence,
        eligible: u64,
        valid: u64,
    ) -> StoreResult<()> {
        if evidence.profile != "cross-section.1"
            || evidence.preprocess_sha256 != self.preprocess
            || evidence.neutralization_sha256 != self.neutralization
            || evidence.exposures_sha256.as_ref()
                != self.exposures.as_ref().map(|value| &value.sha256)
            || evidence.raw_valid_observations > eligible
            || valid > evidence.raw_valid_observations
            || evidence.outcomes.len() != self.sessions
            || evidence.outcomes.iter().any(|outcome| {
                !matches!(
                    outcome.as_str(),
                    "ok" | "insufficient" | "rank_deficient" | "constant"
                )
            })
        {
            return Err(StoreError::Corrupt("transformation result binding"));
        }
        Ok(())
    }
}

fn bind_policy(document: &model::PolicyDocument, expected: &PolicyRef) -> StoreResult<String> {
    model::schema(&document.schema, "loop.research-policy/v1")?;
    let bytes = serde_json::to_vec(document)
        .map_err(|_| StoreError::Corrupt("transformation policy document"))?;
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    if document.policy_id != expected.policy_id().as_str()
        || document.revision != expected.revision().as_str()
        || &digest != expected.sha256()
    {
        return Err(StoreError::Corrupt("transformation policy identity"));
    }
    Ok(format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}
