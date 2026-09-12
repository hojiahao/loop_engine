use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use loop_core::factor::ExpressionId;
use loop_protocol::wire::v1::{
    Actor, ActorId, ActorKind, JobRecord, JobSpecification, job_specification,
};
use prost::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tonic::Request;

use crate::store::{AdmissionPolicy, Clock, StoreError, StoreResult};

pub(super) const CAPABILITY_HEADER: &str = "loop-holdout-capability-bin";

/// Exactly one trust zone per independently provisioned runtime identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Named human operator; cannot act as a numerical worker.
    Operator,
    /// Development-only numerical worker.
    Research,
    /// Dedicated non-LLM protected-evaluation worker.
    HoldoutWorker,
    /// Model-facing discovery process; no generic job or protected API access.
    Discovery,
    /// Provider transport process; no research metadata or artifact access.
    Provider,
    /// Bounded lifecycle recovery identity; no protected artifact access.
    Scheduler,
}

/// Deployment-owned identity mapping. Certificate rotation retains the same
/// human/service subject; different aliases must not count as independent people.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// Stable actor ID, unique within this registry.
    pub actor_id: String,
    /// Stable authenticated person/service identity, never a forwarded header.
    pub subject: String,
    /// Human-readable, non-secret audit label.
    pub display_name: String,
    /// Independent trust zone.
    pub role: Role,
    /// One through eight exact leaf-certificate DER SHA-256 digests.
    pub certificate_sha256: Vec<String>,
    /// Inclusive application validity, checked on every RPC, not only handshake.
    pub not_before_ms: i64,
    /// Exclusive application expiry; certificate validity is also checked by TLS.
    pub expires_at_ms: i64,
    /// Exact authorized run IDs; no wildcard or implicit cross-run access.
    pub run_ids: Vec<String>,
}

impl Identity {
    fn actor(&self) -> Actor {
        Actor {
            actor_id: Some(ActorId {
                value: self.actor_id.clone(),
            }),
            kind: match self.role {
                Role::Operator => ActorKind::Human,
                Role::Discovery => ActorKind::Agent,
                Role::Scheduler => ActorKind::Scheduler,
                _ => ActorKind::Service,
            } as i32,
            display_name: self.display_name.clone(),
            authenticated_subject: self.subject.clone(),
        }
    }
}

/// Pre-approved durable job envelope. This checksum binds administrative bytes;
/// it is not a canonical factor ID or proof of numerical/data semantics.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobPin {
    /// Exact durable job identity.
    pub job_id: String,
    /// SHA-256 of the parsed, persisted JobSpecification's Protobuf encoding.
    pub specification_sha256: String,
}

/// Immutable identity and job registry shared by interceptors and storage policy.
/// No credential values implement Debug or enter error messages. Replacing this
/// registry requires restart and invalidates transient runtime capabilities.
pub struct RuntimeAuthority {
    identities: Vec<Identity>,
    certificates: BTreeMap<[u8; 32], usize>,
    jobs: BTreeMap<String, [u8; 32]>,
    clock: Arc<dyn Clock>,
    last_observed: Mutex<i64>,
}

/// Proof constructed only after tonic exposes a verified TLS peer certificate.
pub(super) struct Principal {
    pub identity: Identity,
    pub actor: Actor,
}

impl RuntimeAuthority {
    /// Validate bounded, deployment-owned identity/job mappings. This constructor
    /// does not authenticate a caller or grant access to any data namespace.
    /// Rejects aliases, malformed digests/IDs, overlapping roles and invalid time
    /// bounds. No mutation, secret generation or filesystem access occurs here.
    pub fn new(
        identities: Vec<Identity>,
        jobs: Vec<JobPin>,
        clock: Arc<dyn Clock>,
    ) -> StoreResult<Self> {
        if identities.is_empty() || identities.len() > 128 || jobs.len() > 4096 {
            return Err(StoreError::Invalid("runtime registry bounds"));
        }
        let mut actors = BTreeSet::new();
        let mut subjects = BTreeSet::new();
        let mut certificates = BTreeMap::new();
        for (index, identity) in identities.iter().enumerate() {
            id(&identity.actor_id)?;
            id(&identity.subject)?;
            let mut runs = BTreeSet::new();
            if identity.display_name.is_empty()
                || identity.display_name.len() > 256
                || identity.display_name.chars().any(char::is_control)
                || !actors.insert(&identity.actor_id)
                || !subjects.insert(&identity.subject)
                || !(1..=8).contains(&identity.certificate_sha256.len())
                || identity.not_before_ms < 0
                || identity.expires_at_ms <= identity.not_before_ms
                || identity.expires_at_ms - identity.not_before_ms > 366 * 86_400_000
                || identity.run_ids.len() > 64
            {
                return Err(StoreError::Invalid("runtime identity"));
            }
            for run in &identity.run_ids {
                id(run)?;
                if !runs.insert(run) {
                    return Err(StoreError::Invalid("duplicate runtime run"));
                }
            }
            for certificate in &identity.certificate_sha256 {
                if certificates.insert(digest(certificate)?, index).is_some() {
                    return Err(StoreError::Invalid("duplicate runtime certificate"));
                }
            }
        }
        let mut pinned = BTreeMap::new();
        for job in jobs {
            id(&job.job_id)?;
            if pinned
                .insert(job.job_id, digest(&job.specification_sha256)?)
                .is_some()
            {
                return Err(StoreError::Invalid("duplicate runtime job"));
            }
        }
        Ok(Self {
            identities,
            certificates,
            jobs: pinned,
            clock,
            last_observed: Mutex::new(0),
        })
    }

    pub(super) fn now(&self) -> StoreResult<i64> {
        let now = self.clock.now_millis()?;
        let mut previous = self
            .last_observed
            .lock()
            .map_err(|_| StoreError::Unavailable("runtime clock"))?;
        if now < *previous {
            return Err(StoreError::ClockRegression);
        }
        *previous = now;
        Ok(now)
    }

    pub(super) fn authenticate<T>(&self, request: &Request<T>) -> StoreResult<Principal> {
        let certificates = request.peer_certs().ok_or(StoreError::AdmissionDenied)?;
        let certificate = certificates.first().ok_or(StoreError::AdmissionDenied)?;
        let index = self
            .certificates
            .get(&<[u8; 32]>::from(Sha256::digest(certificate.as_ref())))
            .ok_or(StoreError::AdmissionDenied)?;
        let identity = self.identities[*index].clone();
        self.check_time(&identity)?;
        // These headers are not a supported authentication/proxy mechanism.
        if [
            "authorization",
            "x-forwarded-client-cert",
            "x-loop-actor",
            "x-loop-subject",
        ]
        .iter()
        .any(|name| request.metadata().contains_key(*name))
            || (request.metadata().contains_key(CAPABILITY_HEADER)
                && identity.role != Role::HoldoutWorker)
        {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(Principal {
            actor: identity.actor(),
            identity,
        })
    }

    fn check_time(&self, identity: &Identity) -> StoreResult<()> {
        let now = self.now()?;
        if now < identity.not_before_ms || now >= identity.expires_at_ms {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }

    pub(super) fn authorize(
        &self,
        principal: &Principal,
        operation: &str,
        job: &JobRecord,
    ) -> StoreResult<()> {
        self.authorize_job_command(operation, &principal.actor, job)
    }

    pub(super) fn authorize_lookup(
        &self,
        principal: &Principal,
        job_id: &str,
        operation: &str,
    ) -> StoreResult<()> {
        let identity = self.identity(&principal.actor)?;
        if !self.jobs.contains_key(job_id)
            || identity.run_ids.is_empty()
            || !role_operation(identity.role, operation)
        {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }

    fn identity(&self, actor: &Actor) -> StoreResult<&Identity> {
        let identity = self
            .identities
            .iter()
            .find(|identity| identity.actor() == *actor)
            .ok_or(StoreError::AdmissionDenied)?;
        self.check_time(identity)?;
        Ok(identity)
    }
}

impl AdmissionPolicy for RuntimeAuthority {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        let job_id = job.job_id.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let expected = self
            .jobs
            .get(&job_id.value)
            .ok_or(StoreError::AdmissionDenied)?;
        if Sha256::digest(job.encode_to_vec()).as_slice() != expected {
            return Err(StoreError::AdmissionDenied);
        }
        let selection = job
            .protocol_selection
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let descriptor = selection
            .schema_descriptor_sha256
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        let features = [
            "jobs.envelope.v1",
            "jobs.kind-input.v1",
            "jobs.prelease-terminal.v1",
        ];
        if selection.selected_package != "loop.v1"
            || descriptor.value != Sha256::digest(loop_protocol::FILE_DESCRIPTOR_SET).as_slice()
            || selection
                .enabled_features
                .iter()
                .any(|feature| !features.contains(&feature.as_str()))
            || features.iter().any(|feature| {
                !selection
                    .enabled_features
                    .iter()
                    .any(|enabled| enabled == feature)
            })
        {
            return Err(StoreError::AdmissionDenied);
        }
        loop_protocol::job::validate_job_specification(job)?;
        Ok(())
    }

    fn authorize_job_command(
        &self,
        operation: &str,
        actor: &Actor,
        record: &JobRecord,
    ) -> StoreResult<()> {
        let identity = self.identity(actor)?;
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        self.validate_submission(job)?;
        let run = job.run_id.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let protected = matches!(
            job.input,
            Some(job_specification::Input::HoldoutBacktest(_))
        );
        let development = matches!(
            job.input,
            Some(
                job_specification::Input::Backtest(_)
                    | job_specification::Input::FactorEvaluation(_)
            )
        );
        let allowed = role_operation(identity.role, operation)
            && match identity.role {
                Role::Operator => !protected,
                Role::Research => development,
                Role::HoldoutWorker => protected,
                Role::Scheduler => true,
                Role::Discovery | Role::Provider => false,
            };
        if !allowed || !identity.run_ids.contains(&run.value) {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }
}

fn role_operation(role: Role, operation: &str) -> bool {
    match role {
        Role::Operator => matches!(
            operation,
            "loop.jobs.read"
                | "loop.jobs.cancel"
                | "loop.backtests.read_current"
                | "loop.backtests.export_current"
        ),
        Role::Research | Role::HoldoutWorker => matches!(
            operation,
            "loop.jobs.read"
                | "loop.jobs.acquire"
                | "loop.jobs.heartbeat"
                | "loop.jobs.complete"
                | "loop.jobs.artifacts"
        ),
        Role::Scheduler => operation == "loop.jobs.recover",
        Role::Discovery | Role::Provider => false,
    }
}

pub(super) fn digest(value: &str) -> StoreResult<[u8; 32]> {
    ExpressionId::parse(value)
        .map(|digest| *digest.as_bytes())
        .map_err(|_| StoreError::Invalid("runtime digest"))
}

fn id(value: &str) -> StoreResult<()> {
    crate::store::validate_id(value)
}
