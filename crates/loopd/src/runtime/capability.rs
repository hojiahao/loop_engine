use std::collections::BTreeMap;
use std::sync::Mutex;

use loop_protocol::wire::v1::JobRecord;
use sha2::{Digest, Sha256};
use tonic::{
    Request,
    metadata::{Binary, MetadataValue},
};

use super::authority::{CAPABILITY_HEADER, Principal, Role};
use crate::store::{StoreError, StoreResult, live_lease};

pub(super) struct Capabilities {
    entries: Mutex<BTreeMap<[u8; 32], Capability>>,
}

struct Capability {
    token: [u8; 32],
    subject: String,
    job_id: String,
    lease_id: String,
    issued_at: i64,
    expires_at: i64,
}

impl Capabilities {
    pub(super) fn command_lease<T>(
        &self,
        request: &Request<T>,
        lease_id: Option<&str>,
    ) -> StoreResult<()> {
        let token = request
            .metadata()
            .get_bin(CAPABILITY_HEADER)
            .ok_or(StoreError::AdmissionDenied)?
            .to_bytes()
            .map_err(|_| StoreError::AdmissionDenied)?;
        let entries = self
            .entries
            .lock()
            .map_err(|_| StoreError::Unavailable("capability state"))?;
        let digest: [u8; 32] = Sha256::digest(token).into();
        if entries.get(&digest).map(|value| value.lease_id.as_str()) != lease_id
            || lease_id.is_none()
        {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }
    pub(super) fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn issue(
        &self,
        principal: &Principal,
        job: &JobRecord,
        now: i64,
    ) -> StoreResult<MetadataValue<Binary>> {
        if principal.identity.role != Role::HoldoutWorker || !super::service::protected(job) {
            return Err(StoreError::AdmissionDenied);
        }
        let job_id = &job
            .specification
            .as_ref()
            .and_then(|spec| spec.job_id.as_ref())
            .ok_or(StoreError::Corrupt("capability job"))?
            .value;
        let lease_id = &job
            .active_lease
            .as_ref()
            .and_then(|lease| lease.lease_id.as_ref())
            .ok_or(StoreError::LeaseFenced)?
            .value;
        let expiry = live_lease(job, &principal.actor, lease_id, now)?
            .min(principal.identity.expires_at_ms)
            .min(now.saturating_add(300_000));
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| StoreError::Unavailable("capability state"))?;
        entries.retain(|_, value| value.expires_at > now);
        let existing = entries.values().find(|value| {
            value.subject == principal.actor.authenticated_subject
                && value.job_id == *job_id
                && value.lease_id == *lease_id
                && value.issued_at <= now
        });
        let token = if let Some(existing) = existing {
            existing.token
        } else {
            if entries.len() >= 4096 {
                return Err(StoreError::Unavailable("capability capacity"));
            }
            let mut token = [0_u8; 32];
            token[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
            token[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
            entries.insert(
                Sha256::digest(token).into(),
                Capability {
                    token,
                    subject: principal.actor.authenticated_subject.clone(),
                    job_id: job_id.clone(),
                    lease_id: lease_id.clone(),
                    issued_at: now,
                    expires_at: expiry,
                },
            );
            token
        };
        let mut metadata = MetadataValue::from_bytes(&token);
        metadata.set_sensitive(true);
        Ok(metadata)
    }

    pub(super) fn check<T>(
        &self,
        request: &Request<T>,
        principal: &Principal,
        job: &JobRecord,
        now: i64,
        historical_retry: bool,
    ) -> StoreResult<()> {
        if principal.identity.role != Role::HoldoutWorker {
            return Err(StoreError::AdmissionDenied);
        }
        let values: Vec<_> = request
            .metadata()
            .get_all_bin(CAPABILITY_HEADER)
            .iter()
            .collect();
        if values.len() != 1 {
            return Err(StoreError::AdmissionDenied);
        }
        let token = values[0]
            .to_bytes()
            .map_err(|_| StoreError::AdmissionDenied)?;
        if token.len() != 32 {
            return Err(StoreError::AdmissionDenied);
        }
        let entries = self
            .entries
            .lock()
            .map_err(|_| StoreError::Unavailable("capability state"))?;
        let digest: [u8; 32] = Sha256::digest(token).into();
        let capability = entries.get(&digest).ok_or(StoreError::AdmissionDenied)?;
        if capability.subject != principal.actor.authenticated_subject
            || job
                .specification
                .as_ref()
                .and_then(|spec| spec.job_id.as_ref())
                .map(|id| id.value.as_str())
                != Some(capability.job_id.as_str())
            || now < capability.issued_at
            || now >= capability.expires_at
        {
            return Err(StoreError::AdmissionDenied);
        }
        let terminal = matches!(
            loop_protocol::wire::v1::JobState::try_from(job.state),
            Ok(loop_protocol::wire::v1::JobState::Succeeded
                | loop_protocol::wire::v1::JobState::FactorRejected
                | loop_protocol::wire::v1::JobState::InfrastructureFailed
                | loop_protocol::wire::v1::JobState::Cancelled
                | loop_protocol::wire::v1::JobState::BudgetExhausted)
        );
        if !(historical_retry && terminal) {
            live_lease(job, &principal.actor, &capability.lease_id, now)?;
        }
        Ok(())
    }
}
