use std::sync::{Arc, atomic::AtomicI64};

use loop_protocol::wire::v1::{JobLease, JobRecord, JobState, LeaseId, job_specification};
use tonic::{Request, metadata::MetadataValue};

use super::authority::{CAPABILITY_HEADER, Principal};
use super::capability::Capabilities;
use super::{Identity, Role, RuntimeAuthority};
use crate::store::{AdmissionPolicy, StoreError};
use crate::test_support::{self as support, NOW, actor, timestamp};

fn identity() -> Identity {
    Identity {
        actor_id: "worker".to_owned(),
        subject: "service:worker".to_owned(),
        display_name: "Worker".to_owned(),
        role: Role::HoldoutWorker,
        certificate_sha256: vec![format!("sha256:{}", "01".repeat(32))],
        not_before_ms: NOW - 1000,
        expires_at_ms: NOW + 60_000,
        run_ids: vec!["run.1".to_owned()],
    }
}

fn registry(identities: Vec<Identity>) -> Result<RuntimeAuthority, StoreError> {
    RuntimeAuthority::new(
        identities,
        vec![],
        Arc::new(support::FixtureClock(AtomicI64::new(NOW))),
    )
}

#[test]
fn certificate_aliases_are_denied() {
    let first = identity();
    let mut alias = first.clone();
    alias.actor_id = "alias".to_owned();
    alias.subject = "service:alias".to_owned();
    assert!(registry(vec![first, alias]).is_err());
}

#[test]
fn subject_aliases_are_not_independent_identities() {
    let first = identity();
    let mut alias = first.clone();
    alias.actor_id = "alias".to_owned();
    alias.certificate_sha256 = vec![format!("sha256:{}", "02".repeat(32))];
    assert!(registry(vec![first, alias]).is_err());
}

#[test]
fn unpinned_job_envelopes_are_denied() {
    assert!(matches!(
        registry(vec![identity()])
            .unwrap()
            .validate_submission(&support::command(1).specification),
        Err(StoreError::AdmissionDenied)
    ));
}

fn leased() -> (Principal, JobRecord) {
    let mut specification = support::command(1).specification;
    specification.input = Some(job_specification::Input::HoldoutBacktest(Default::default()));
    let principal = Principal {
        identity: identity(),
        actor: actor(),
    };
    let job = JobRecord {
        specification: Some(specification),
        state: JobState::Leased as i32,
        revision: 2,
        active_lease: Some(JobLease {
            lease_id: Some(LeaseId {
                value: "lease.1".to_owned(),
            }),
            owner: Some(actor()),
            issued_at: Some(timestamp(NOW)),
            expires_at: Some(timestamp(NOW + 10_000)),
            ..Default::default()
        }),
        ..Default::default()
    };
    (principal, job)
}

fn request(token: MetadataValue<tonic::metadata::Binary>) -> Request<()> {
    let mut request = Request::new(());
    request.metadata_mut().insert_bin(CAPABILITY_HEADER, token);
    request
}

#[test]
fn capability_is_bound_to_subject() {
    let (mut principal, job) = leased();
    let capabilities = Capabilities::new();
    let request = request(capabilities.issue(&principal, &job, NOW).unwrap());
    principal.actor.authenticated_subject = "other".to_owned();
    assert!(
        capabilities
            .check(&request, &principal, &job, NOW, false)
            .is_err()
    );
}

#[test]
fn capability_is_bound_to_job() {
    let (principal, mut job) = leased();
    let capabilities = Capabilities::new();
    let request = request(capabilities.issue(&principal, &job, NOW).unwrap());
    job.specification
        .as_mut()
        .unwrap()
        .job_id
        .as_mut()
        .unwrap()
        .value = "job.other".to_owned();
    assert!(
        capabilities
            .check(&request, &principal, &job, NOW, false)
            .is_err()
    );
}

#[test]
fn capability_does_not_follow_a_replacement_lease() {
    let (principal, mut job) = leased();
    let capabilities = Capabilities::new();
    let request = request(capabilities.issue(&principal, &job, NOW).unwrap());
    job.active_lease
        .as_mut()
        .unwrap()
        .lease_id
        .as_mut()
        .unwrap()
        .value = "lease.2".to_owned();
    assert!(
        capabilities
            .check(&request, &principal, &job, NOW, false)
            .is_err()
    );
}

#[test]
fn capability_expiry_is_exclusive() {
    let (principal, job) = leased();
    let capabilities = Capabilities::new();
    let token = capabilities.issue(&principal, &job, NOW).unwrap();
    assert!(token.is_sensitive());
    let request = request(token);
    assert!(
        capabilities
            .check(&request, &principal, &job, NOW + 9_999, false)
            .is_ok()
    );
    assert!(
        capabilities
            .check(&request, &principal, &job, NOW + 10_000, false)
            .is_err()
    );
}

#[test]
fn terminal_retry_does_not_authorize_data() {
    let (principal, mut job) = leased();
    let capabilities = Capabilities::new();
    let request = request(capabilities.issue(&principal, &job, NOW).unwrap());
    job.state = JobState::InfrastructureFailed as i32;
    job.active_lease = None;
    assert!(
        capabilities
            .check(&request, &principal, &job, NOW, true)
            .is_ok()
    );
    assert!(
        capabilities
            .check(&request, &principal, &job, NOW, false)
            .is_err()
    );
    assert!(
        capabilities
            .command_lease(&request, Some("lease.2"))
            .is_err()
    );
}

#[test]
fn operational_errors_have_typed_redacted_details() {
    use loop_protocol::runtime_validation::{RichStatusDetail, validate_operational_failure};
    use prost::Message;
    let status = super::service::status(StoreError::Corrupt("private path or secret"));
    let rich = super::service::RichStatus::decode(status.details()).unwrap();
    assert_eq!(rich.code, status.code() as i32);
    assert_eq!(rich.message, status.message());
    let details = rich
        .details
        .iter()
        .map(|value| RichStatusDetail {
            type_url: &value.type_url,
            value: &value.value,
        })
        .collect::<Vec<_>>();
    let error = validate_operational_failure(rich.code, &[], &details).unwrap();
    assert_eq!(error.code, "evidence_unavailable");
    assert!(!format!("{status:?}").contains("private path or secret"));
}
