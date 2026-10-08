//! A single pinned discovery step; no supplier logic or factor admission.

mod execution;
mod plan;
mod projection;
pub(super) use projection::metadata;
#[cfg(test)]
mod tests;
mod tools;
mod transport;

use std::path::{Path, PathBuf};

use loop_protocol::wire::{discovery::v1 as wire, v1};
use serde::Deserialize;

use self::plan::FrozenPlan;
use self::transport::ProviderConnection;
use crate::manifests::{LocalArtifacts, ObjectRef};
use crate::store::{StoreError, StoreResult};

/// Explicit bounded execution deployment. Absent configuration retains owner
/// stop/status/events while denying execution and candidate reads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryConfig {
    /// Separate immutable prompt/plan namespace, never the protected data store.
    pub plan_store: PathBuf,
    /// One through 64 administrator-pinned content-addressed plans.
    pub plans: Vec<ObjectRef>,
    /// Private mTLS connector; contains no supplier credential.
    pub provider: ProviderConfig,
}

/// A single authenticated Provider endpoint and its exact caller identity.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// HTTPS gRPC endpoint; no URL userinfo, query or fragment is accepted.
    pub endpoint: String,
    /// TLS server name bound by the trusted CA.
    pub domain: String,
    /// Exact Discovery actor registered at both services.
    pub actor_id: String,
    /// Stable authenticated service subject, not forwarded RPC metadata.
    pub subject: String,
    /// Non-secret registered audit label.
    pub display_name: String,
    /// Provider CA bundle.
    pub ca_file: PathBuf,
    /// Runtime client certificate.
    pub certificate_file: PathBuf,
    /// Private client key; never hashed into public plan identity or logged.
    pub private_key_file: PathBuf,
}

/// Frozen plans and development-only data access for one-step discovery.
/// Opening validates configuration without connecting to a Provider or database.
pub struct DiscoveryExecutor {
    plans: Vec<FrozenPlan>,
    data: LocalArtifacts,
    provider: ProviderConnection,
}

impl DiscoveryExecutor {
    /// Load and bind immutable plans to the installed codec and mTLS connector.
    /// Missing objects, unknown schemas, overlapping namespaces and drift deny
    /// startup. No files, credentials or reservations are created here.
    pub fn open(config: DiscoveryConfig, development: &Path) -> StoreResult<Self> {
        if config.plans.is_empty()
            || config.plans.len() > 64
            || super::reconciliation::overlaps(&config.plan_store, development)
        {
            return Err(StoreError::Invalid("discovery plan bounds or namespace"));
        }
        let provider = ProviderConnection::open(config.provider)?;
        let mut plans: Vec<FrozenPlan> = Vec::new();
        for reference in config.plans {
            let plan = FrozenPlan::load(&config.plan_store, &reference)?;
            if plan.provider_sha256 != provider.sha256
                || plan.actor_id
                    != provider
                        .actor
                        .actor_id
                        .as_ref()
                        .ok_or(StoreError::Invalid("provider actor"))?
                        .value
                || plans
                    .iter()
                    .any(|existing| existing.input.research_policy == plan.input.research_policy)
            {
                return Err(StoreError::AdmissionDenied);
            }
            plans.push(plan);
        }
        Ok(Self {
            plans,
            data: LocalArtifacts::open(development)?,
            provider,
        })
    }

    pub(super) fn identities(&self) -> impl Iterator<Item = (&v1::Actor, &str)> {
        self.plans
            .iter()
            .map(|plan| (&self.provider.actor, plan.run_id.as_str()))
    }

    pub(super) fn submission(
        &self,
        actor: &v1::Actor,
        input: &wire::DiscoveryJobInput,
    ) -> StoreResult<crate::store::SubmissionMetadata> {
        let plan = self
            .plans
            .iter()
            .find(|plan| plan.input == *input)
            .ok_or(StoreError::AdmissionDenied)?;
        plan.check()?;
        if actor != &self.provider.actor {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(crate::store::SubmissionMetadata {
            run_id: v1::RunId {
                value: plan.run_id.clone(),
            },
            protocol_selection: plan.protocol.clone(),
        })
    }

    pub(super) fn authorize(&self, job: &v1::JobSpecification) -> StoreResult<()> {
        self.plan(job).map(|_| ())
    }

    fn plan(&self, job: &v1::JobSpecification) -> StoreResult<&FrozenPlan> {
        if job.submitted_by.as_ref() != Some(&self.provider.actor) {
            return Err(StoreError::AdmissionDenied);
        }
        let Some(v1::job_specification::Input::Discovery(input)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        let plan = self
            .plans
            .iter()
            .find(|plan| plan.input.research_policy == input.research_policy)
            .ok_or(StoreError::AdmissionDenied)?;
        plan.matches(job)?;
        Ok(plan)
    }
}
