use std::collections::HashSet;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    AuditAction, AuditError, AuditErrorCode, AuditEvent, AuditPayload, AuditTargetKind,
    MAX_AUDIT_PAYLOAD_BYTES, Sha256Digest, validate_domain_id, validate_schema_name,
    validate_target, validate_text, validate_timestamp,
};

const PAYLOAD_DOMAIN: &[u8] = b"loop.audit-payload/v1\0";
const EVENT_DOMAIN: &[u8] = b"loop.audit-event/v1\0";
const EVENT_SCHEMA: &str = "loop.audit-event/v1";
const MAX_HOLDOUT_APPROVAL_RECORDS: usize = 8;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandAcceptedPayload {
    command: String,
    request_id: String,
    summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateTransitionedPayload {
    from: String,
    to: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactorAdmittedPayload {
    factor_spec_id: String,
    decision: String,
    evidence_artifact_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactorRejectedPayload {
    factor_spec_id: String,
    rejection_code: String,
    reason: String,
    evidence_artifact_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverrideAuthorizedPayload {
    factor_spec_id: String,
    override_kind: String,
    authorized_by_actor_id: String,
    reason: String,
    approval_reference: String,
    evidence_artifact_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadmissionRequestedPayload {
    factor_spec_id: String,
    original_rejection_event_id: String,
    requested_by_actor_id: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadmissionDecidedPayload {
    factor_spec_id: String,
    original_rejection_event_id: String,
    disposition: String,
    decided_by_actor_id: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldoutGrantIssuedPayload {
    holdout_grant_id: String,
    holdout_period_id: String,
    freeze_manifest_sha256: String,
    holdout_evaluation_plan_id: String,
    approval_records: Vec<HoldoutGrantApprovalRecord>,
    capability_class: String,
    authorization_decision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldoutGrantApprovalRecord {
    holdout_approval_record_id: String,
    approval_record_sha256: String,
    approved_by_actor_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldoutGrantConsumedPayload {
    holdout_grant_id: String,
    holdout_period_id: String,
    holdout_evaluation_plan_id: String,
    job_batch_id: String,
    capability_class: String,
    authorization_decision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactExportedPayload {
    artifact_id: String,
    export_class: String,
    policy_id: String,
    destination_class: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldoutApprovalRecordedPayload {
    holdout_approval_record_id: String,
    holdout_period_id: String,
    freeze_manifest_sha256: String,
    approved_by_actor_id: String,
    expires_at: String,
}

struct RegisteredPayload {
    rewritten: Vec<u8>,
}

pub fn canonicalize_audit_payload(
    schema_name: &str,
    schema_version: u32,
    submitted_bytes: &[u8],
) -> Result<AuditPayload, AuditError> {
    validate_schema_name(schema_name)?;
    if schema_version == 0 {
        return Err(AuditError::new(
            AuditErrorCode::InvalidSchema,
            "payload.schema_version",
            "schema version must be positive",
        ));
    }
    if submitted_bytes.len() > MAX_AUDIT_PAYLOAD_BYTES {
        return Err(AuditError::new(
            AuditErrorCode::SizeLimit,
            "payload.canonical_bytes",
            "canonical payload exceeds 256 KiB",
        ));
    }

    let registered = match (schema_name, schema_version) {
        ("loop.audit.command_accepted", 1) => {
            let payload: CommandAcceptedPayload = parse_payload(submitted_bytes)?;
            validate_schema_name(&payload.command).map_err(|_| {
                AuditError::new(
                    AuditErrorCode::InvalidSchema,
                    "payload.command",
                    "command must be a dot-qualified lowercase ASCII identifier",
                )
            })?;
            validate_domain_id(&payload.request_id, "payload.request_id")?;
            validate_text(&payload.summary, "payload.summary", true)?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("command", &payload.command),
                    ("request_id", &payload.request_id),
                    ("summary", &payload.summary),
                ]),
            }
        }
        ("loop.audit.state_transitioned", 1) => {
            let payload: StateTransitionedPayload = parse_payload(submitted_bytes)?;
            validate_schema_name(&payload.from).map_err(|_| {
                AuditError::new(
                    AuditErrorCode::InvalidSchema,
                    "payload.from",
                    "state must be a canonical lowercase identifier",
                )
            })?;
            validate_schema_name(&payload.to).map_err(|_| {
                AuditError::new(
                    AuditErrorCode::InvalidSchema,
                    "payload.to",
                    "state must be a canonical lowercase identifier",
                )
            })?;
            validate_text(&payload.reason, "payload.reason", true)?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("from", &payload.from),
                    ("to", &payload.to),
                    ("reason", &payload.reason),
                ]),
            }
        }
        ("loop.audit.factor_admitted", 1) => {
            let payload: FactorAdmittedPayload = parse_payload(submitted_bytes)?;
            validate_sha256_field(&payload.factor_spec_id, "payload.factor_spec_id")?;
            validate_closed_enum(&payload.decision, &["admitted"], "payload.decision")?;
            validate_sha256_field(
                &payload.evidence_artifact_id,
                "payload.evidence_artifact_id",
            )?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("factor_spec_id", &payload.factor_spec_id),
                    ("decision", &payload.decision),
                    ("evidence_artifact_id", &payload.evidence_artifact_id),
                ]),
            }
        }
        ("loop.audit.factor_rejected", 1) => {
            let payload: FactorRejectedPayload = parse_payload(submitted_bytes)?;
            validate_sha256_field(&payload.factor_spec_id, "payload.factor_spec_id")?;
            validate_closed_enum(
                &payload.rejection_code,
                &[
                    "duplicate",
                    "previously_failed",
                    "insufficient_coverage",
                    "deterministic_filter",
                    "performance",
                    "correlation",
                    "semantic_review",
                    "policy",
                ],
                "payload.rejection_code",
            )?;
            validate_text(&payload.reason, "payload.reason", true)?;
            validate_sha256_field(
                &payload.evidence_artifact_id,
                "payload.evidence_artifact_id",
            )?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("factor_spec_id", &payload.factor_spec_id),
                    ("rejection_code", &payload.rejection_code),
                    ("reason", &payload.reason),
                    ("evidence_artifact_id", &payload.evidence_artifact_id),
                ]),
            }
        }
        ("loop.audit.override_authorized", 1) => {
            let payload: OverrideAuthorizedPayload = parse_payload(submitted_bytes)?;
            validate_sha256_field(&payload.factor_spec_id, "payload.factor_spec_id")?;
            validate_closed_enum(
                &payload.override_kind,
                &["force_admission", "readmission", "policy_exception"],
                "payload.override_kind",
            )?;
            validate_domain_id(
                &payload.authorized_by_actor_id,
                "payload.authorized_by_actor_id",
            )?;
            validate_text(&payload.reason, "payload.reason", true)?;
            validate_domain_id(&payload.approval_reference, "payload.approval_reference")?;
            validate_sha256_field(
                &payload.evidence_artifact_id,
                "payload.evidence_artifact_id",
            )?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("factor_spec_id", &payload.factor_spec_id),
                    ("override_kind", &payload.override_kind),
                    ("authorized_by_actor_id", &payload.authorized_by_actor_id),
                    ("reason", &payload.reason),
                    ("approval_reference", &payload.approval_reference),
                    ("evidence_artifact_id", &payload.evidence_artifact_id),
                ]),
            }
        }
        ("loop.audit.readmission_requested", 1) => {
            let payload: ReadmissionRequestedPayload = parse_payload(submitted_bytes)?;
            validate_sha256_field(&payload.factor_spec_id, "payload.factor_spec_id")?;
            validate_domain_id(
                &payload.original_rejection_event_id,
                "payload.original_rejection_event_id",
            )?;
            validate_domain_id(
                &payload.requested_by_actor_id,
                "payload.requested_by_actor_id",
            )?;
            validate_text(&payload.reason, "payload.reason", true)?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("factor_spec_id", &payload.factor_spec_id),
                    (
                        "original_rejection_event_id",
                        &payload.original_rejection_event_id,
                    ),
                    ("requested_by_actor_id", &payload.requested_by_actor_id),
                    ("reason", &payload.reason),
                ]),
            }
        }
        ("loop.audit.readmission_decided", 1) => {
            let payload: ReadmissionDecidedPayload = parse_payload(submitted_bytes)?;
            validate_sha256_field(&payload.factor_spec_id, "payload.factor_spec_id")?;
            validate_domain_id(
                &payload.original_rejection_event_id,
                "payload.original_rejection_event_id",
            )?;
            validate_closed_enum(
                &payload.disposition,
                &["admitted", "rejected", "quarantined"],
                "payload.disposition",
            )?;
            validate_domain_id(&payload.decided_by_actor_id, "payload.decided_by_actor_id")?;
            validate_text(&payload.reason, "payload.reason", true)?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("factor_spec_id", &payload.factor_spec_id),
                    (
                        "original_rejection_event_id",
                        &payload.original_rejection_event_id,
                    ),
                    ("disposition", &payload.disposition),
                    ("decided_by_actor_id", &payload.decided_by_actor_id),
                    ("reason", &payload.reason),
                ]),
            }
        }
        ("loop.audit.holdout_grant_issued", 1) => {
            let payload: HoldoutGrantIssuedPayload = parse_payload(submitted_bytes)?;
            validate_domain_id(&payload.holdout_grant_id, "payload.holdout_grant_id")?;
            validate_sha256_field(&payload.holdout_period_id, "payload.holdout_period_id")?;
            validate_sha256_field(
                &payload.freeze_manifest_sha256,
                "payload.freeze_manifest_sha256",
            )?;
            validate_sha256_field(
                &payload.holdout_evaluation_plan_id,
                "payload.holdout_evaluation_plan_id",
            )?;
            validate_holdout_approval_records(&payload.approval_records)?;
            validate_closed_enum(
                &payload.capability_class,
                &["holdout_evaluation"],
                "payload.capability_class",
            )?;
            validate_closed_enum(
                &payload.authorization_decision,
                &["authorized"],
                "payload.authorization_decision",
            )?;
            RegisteredPayload {
                rewritten: write_holdout_grant_issued_payload(&payload),
            }
        }
        ("loop.audit.holdout_grant_consumed", 1) => {
            let payload: HoldoutGrantConsumedPayload = parse_payload(submitted_bytes)?;
            validate_domain_id(&payload.holdout_grant_id, "payload.holdout_grant_id")?;
            validate_sha256_field(&payload.holdout_period_id, "payload.holdout_period_id")?;
            validate_sha256_field(
                &payload.holdout_evaluation_plan_id,
                "payload.holdout_evaluation_plan_id",
            )?;
            validate_domain_id(&payload.job_batch_id, "payload.job_batch_id")?;
            validate_closed_enum(
                &payload.capability_class,
                &["holdout_evaluation"],
                "payload.capability_class",
            )?;
            validate_closed_enum(
                &payload.authorization_decision,
                &["authorized"],
                "payload.authorization_decision",
            )?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("holdout_grant_id", &payload.holdout_grant_id),
                    ("holdout_period_id", &payload.holdout_period_id),
                    (
                        "holdout_evaluation_plan_id",
                        &payload.holdout_evaluation_plan_id,
                    ),
                    ("job_batch_id", &payload.job_batch_id),
                    ("capability_class", &payload.capability_class),
                    ("authorization_decision", &payload.authorization_decision),
                ]),
            }
        }
        ("loop.audit.artifact_exported", 1) => {
            let payload: ArtifactExportedPayload = parse_payload(submitted_bytes)?;
            validate_sha256_field(&payload.artifact_id, "payload.artifact_id")?;
            validate_closed_enum(
                &payload.export_class,
                &[
                    "research_report",
                    "audit_bundle",
                    "data_snapshot",
                    "factor_values",
                ],
                "payload.export_class",
            )?;
            validate_sha256_field(&payload.policy_id, "payload.policy_id")?;
            validate_closed_enum(
                &payload.destination_class,
                &["local_managed", "approved_object_store", "user_download"],
                "payload.destination_class",
            )?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    ("artifact_id", &payload.artifact_id),
                    ("export_class", &payload.export_class),
                    ("policy_id", &payload.policy_id),
                    ("destination_class", &payload.destination_class),
                ]),
            }
        }
        ("loop.audit.holdout_approval_recorded", 1) => {
            let payload: HoldoutApprovalRecordedPayload = parse_payload(submitted_bytes)?;
            validate_domain_id(
                &payload.holdout_approval_record_id,
                "payload.holdout_approval_record_id",
            )?;
            validate_sha256_field(&payload.holdout_period_id, "payload.holdout_period_id")?;
            validate_sha256_field(
                &payload.freeze_manifest_sha256,
                "payload.freeze_manifest_sha256",
            )?;
            validate_domain_id(
                &payload.approved_by_actor_id,
                "payload.approved_by_actor_id",
            )?;
            validate_timestamp_field(&payload.expires_at, "payload.expires_at")?;
            RegisteredPayload {
                rewritten: write_payload_fields(&[
                    (
                        "holdout_approval_record_id",
                        &payload.holdout_approval_record_id,
                    ),
                    ("holdout_period_id", &payload.holdout_period_id),
                    ("freeze_manifest_sha256", &payload.freeze_manifest_sha256),
                    ("approved_by_actor_id", &payload.approved_by_actor_id),
                    ("expires_at", &payload.expires_at),
                ]),
            }
        }
        _ => {
            return Err(AuditError::new(
                AuditErrorCode::UnsupportedSchema,
                "payload.schema_name",
                "audit payload schema/version is not registered",
            ));
        }
    };

    if registered.rewritten != submitted_bytes {
        return Err(AuditError::new(
            AuditErrorCode::NonCanonicalPayload,
            "payload.canonical_bytes",
            "payload bytes differ from the registered dedicated writer",
        ));
    }
    let payload_sha256 = audit_payload_sha256(schema_name, schema_version, &registered.rewritten)?;
    Ok(AuditPayload {
        schema_name: schema_name.to_owned(),
        schema_version,
        canonical_bytes: registered.rewritten,
        payload_sha256,
    })
}

pub fn audit_payload_sha256(
    schema_name: &str,
    schema_version: u32,
    canonical_payload_bytes: &[u8],
) -> Result<Sha256Digest, AuditError> {
    validate_schema_name(schema_name)?;
    if schema_version == 0 {
        return Err(AuditError::new(
            AuditErrorCode::InvalidSchema,
            "payload.schema_version",
            "schema version must be positive",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(PAYLOAD_DOMAIN);
    hasher.update(schema_name.as_bytes());
    hasher.update([0]);
    hasher.update(schema_version.to_string().as_bytes());
    hasher.update([0]);
    hasher.update(canonical_payload_bytes);
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

pub fn verify_audit_payload(payload: &AuditPayload) -> Result<(), AuditError> {
    let verified = canonicalize_audit_payload(
        &payload.schema_name,
        payload.schema_version,
        &payload.canonical_bytes,
    )?;
    if verified.payload_sha256 != payload.payload_sha256 {
        return Err(AuditError::new(
            AuditErrorCode::PayloadDigestMismatch,
            "payload.payload_sha256",
            "claimed payload digest does not match canonical payload bytes",
        ));
    }
    Ok(())
}

pub fn canonical_audit_event_bytes(event: &AuditEvent) -> Result<Vec<u8>, AuditError> {
    verify_audit_payload(&event.payload)?;
    validate_event_fields(event)?;

    let mut output = Vec::new();
    output.extend_from_slice(b"{\"schema\":\"");
    output.extend_from_slice(EVENT_SCHEMA.as_bytes());
    output.extend_from_slice(b"\",\"audit_ledger_id\":");
    write_json_string(&mut output, &event.audit_ledger_id);
    output.extend_from_slice(b",\"sequence\":\"");
    output.extend_from_slice(event.sequence.to_string().as_bytes());
    output.extend_from_slice(b"\",\"previous_event_sha256\":\"");
    output.extend_from_slice(event.previous_event_sha256.to_string().as_bytes());
    output.extend_from_slice(b"\",\"audit_event_id\":");
    write_json_string(&mut output, &event.audit_event_id);
    output.extend_from_slice(b",\"occurred_at\":");
    write_json_string(&mut output, &event.occurred_at);
    output.extend_from_slice(b",\"correlation_id\":");
    write_json_string(&mut output, &event.correlation_id);
    output.extend_from_slice(b",\"causation_id\":");
    write_json_string(&mut output, &event.causation_id);
    output.extend_from_slice(b",\"actor\":{\"actor_id\":");
    write_json_string(&mut output, &event.actor.actor_id);
    output.extend_from_slice(b",\"kind\":\"");
    output.extend_from_slice(event.actor.kind.as_str().as_bytes());
    output.extend_from_slice(b"\",\"display_name\":");
    write_json_string(&mut output, &event.actor.display_name);
    output.extend_from_slice(b",\"authenticated_subject\":");
    write_json_string(&mut output, &event.actor.authenticated_subject);
    output.extend_from_slice(b"},\"action\":\"");
    output.extend_from_slice(event.action.as_str().as_bytes());
    output.extend_from_slice(b"\",\"target\":{\"kind\":\"");
    output.extend_from_slice(event.target.kind.as_str().as_bytes());
    output.extend_from_slice(b"\",\"value\":");
    write_json_string(&mut output, &event.target.value);
    output.extend_from_slice(b"},\"payload\":{\"schema_name\":\"");
    output.extend_from_slice(event.payload.schema_name.as_bytes());
    output.extend_from_slice(b"\",\"schema_version\":\"");
    output.extend_from_slice(event.payload.schema_version.to_string().as_bytes());
    output.extend_from_slice(b"\",\"payload_sha256\":\"");
    output.extend_from_slice(event.payload.payload_sha256.to_string().as_bytes());
    output.extend_from_slice(b"\"}}");
    Ok(output)
}

pub fn audit_event_sha256(event: &AuditEvent) -> Result<Sha256Digest, AuditError> {
    let canonical = canonical_audit_event_bytes(event)?;
    let mut hasher = Sha256::new();
    hasher.update(EVENT_DOMAIN);
    hasher.update(canonical);
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

pub fn verify_audit_event(event: &AuditEvent) -> Result<(), AuditError> {
    let computed = audit_event_sha256(event)?;
    if computed != event.event_sha256 {
        return Err(AuditError::new(
            AuditErrorCode::EventDigestMismatch,
            "event_sha256",
            "claimed event digest does not match canonical event bytes",
        ));
    }
    Ok(())
}

pub fn verify_audit_chain(events: &[AuditEvent]) -> Result<(), AuditError> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let ledger_id = first.audit_ledger_id.as_str();
    let mut previous = Sha256Digest::ZERO;
    let mut expected_sequence = 1_u64;
    let mut event_ids = HashSet::with_capacity(events.len());

    for event in events {
        verify_audit_event(event)?;
        if event.audit_ledger_id != ledger_id {
            return Err(AuditError::new(
                AuditErrorCode::LedgerMismatch,
                "audit_ledger_id",
                "all events in one verified chain must use the same ledger ID",
            ));
        }
        if event.sequence != expected_sequence {
            return Err(AuditError::new(
                AuditErrorCode::InvalidSequence,
                "sequence",
                "audit sequence must begin at one and increase exactly by one",
            ));
        }
        if event.previous_event_sha256 != previous {
            return Err(AuditError::new(
                AuditErrorCode::ChainMismatch,
                "previous_event_sha256",
                "event does not commit to the immediately preceding event digest",
            ));
        }
        if !event_ids.insert(event.audit_event_id.as_str()) {
            return Err(AuditError::new(
                AuditErrorCode::DuplicateEventId,
                "audit_event_id",
                "audit event IDs must be unique within a chain",
            ));
        }
        previous = event.event_sha256;
        expected_sequence = expected_sequence.checked_add(1).ok_or_else(|| {
            AuditError::new(
                AuditErrorCode::InvalidSequence,
                "sequence",
                "audit sequence overflowed uint64",
            )
        })?;
    }
    Ok(())
}

fn validate_event_fields(event: &AuditEvent) -> Result<(), AuditError> {
    validate_domain_id(&event.audit_ledger_id, "audit_ledger_id")?;
    if event.sequence == 0 {
        return Err(AuditError::new(
            AuditErrorCode::InvalidSequence,
            "sequence",
            "event sequence must be positive",
        ));
    }
    validate_domain_id(&event.audit_event_id, "audit_event_id")?;
    validate_timestamp(&event.occurred_at)?;
    validate_domain_id(&event.correlation_id, "correlation_id")?;
    validate_domain_id(&event.causation_id, "causation_id")?;
    validate_domain_id(&event.actor.actor_id, "actor.actor_id")?;
    validate_text(&event.actor.display_name, "actor.display_name", false)?;
    validate_text(
        &event.actor.authenticated_subject,
        "actor.authenticated_subject",
        true,
    )?;
    validate_target(&event.target)?;
    validate_action_binding(event)
}

fn validate_action_binding(event: &AuditEvent) -> Result<(), AuditError> {
    let expected_schema = match event.action {
        AuditAction::CommandAccepted => "loop.audit.command_accepted",
        AuditAction::StateTransitioned => "loop.audit.state_transitioned",
        AuditAction::FactorAdmitted => "loop.audit.factor_admitted",
        AuditAction::FactorRejected => "loop.audit.factor_rejected",
        AuditAction::OverrideAuthorized => "loop.audit.override_authorized",
        AuditAction::ReadmissionRequested => "loop.audit.readmission_requested",
        AuditAction::ReadmissionDecided => "loop.audit.readmission_decided",
        AuditAction::HoldoutGrantIssued => "loop.audit.holdout_grant_issued",
        AuditAction::HoldoutGrantConsumed => "loop.audit.holdout_grant_consumed",
        AuditAction::ArtifactExported => "loop.audit.artifact_exported",
        AuditAction::HoldoutApprovalRecorded => "loop.audit.holdout_approval_recorded",
    };
    if event.payload.schema_name != expected_schema || event.payload.schema_version != 1 {
        return Err(AuditError::new(
            AuditErrorCode::ActionPayloadMismatch,
            "action",
            "audit action is not bound to the payload schema/version",
        ));
    }

    let target_is_allowed = match event.action {
        AuditAction::CommandAccepted => matches!(
            event.target.kind,
            AuditTargetKind::RunId
                | AuditTargetKind::JobId
                | AuditTargetKind::FactorSpecId
                | AuditTargetKind::BacktestId
                | AuditTargetKind::SnapshotId
                | AuditTargetKind::ArtifactId
        ),
        AuditAction::StateTransitioned => matches!(
            event.target.kind,
            AuditTargetKind::RunId
                | AuditTargetKind::JobId
                | AuditTargetKind::BacktestId
                | AuditTargetKind::SnapshotId
        ),
        AuditAction::FactorAdmitted
        | AuditAction::FactorRejected
        | AuditAction::OverrideAuthorized
        | AuditAction::ReadmissionRequested
        | AuditAction::ReadmissionDecided => event.target.kind == AuditTargetKind::FactorSpecId,
        AuditAction::HoldoutGrantIssued | AuditAction::HoldoutGrantConsumed => {
            event.target.kind == AuditTargetKind::HoldoutGrantId
        }
        AuditAction::ArtifactExported => event.target.kind == AuditTargetKind::ArtifactId,
        AuditAction::HoldoutApprovalRecorded => {
            event.target.kind == AuditTargetKind::HoldoutApprovalRecordId
        }
    };
    if !target_is_allowed {
        return Err(AuditError::new(
            AuditErrorCode::ActionTargetMismatch,
            "target.kind",
            "audit action does not permit this target kind",
        ));
    }

    let subject = match event.action {
        AuditAction::CommandAccepted | AuditAction::StateTransitioned => None,
        AuditAction::FactorAdmitted => Some(
            parse_payload::<FactorAdmittedPayload>(&event.payload.canonical_bytes)?.factor_spec_id,
        ),
        AuditAction::FactorRejected => Some(
            parse_payload::<FactorRejectedPayload>(&event.payload.canonical_bytes)?.factor_spec_id,
        ),
        AuditAction::OverrideAuthorized => Some(
            parse_payload::<OverrideAuthorizedPayload>(&event.payload.canonical_bytes)?
                .factor_spec_id,
        ),
        AuditAction::ReadmissionRequested => Some(
            parse_payload::<ReadmissionRequestedPayload>(&event.payload.canonical_bytes)?
                .factor_spec_id,
        ),
        AuditAction::ReadmissionDecided => Some(
            parse_payload::<ReadmissionDecidedPayload>(&event.payload.canonical_bytes)?
                .factor_spec_id,
        ),
        AuditAction::HoldoutGrantIssued => Some(
            parse_payload::<HoldoutGrantIssuedPayload>(&event.payload.canonical_bytes)?
                .holdout_grant_id,
        ),
        AuditAction::HoldoutGrantConsumed => Some(
            parse_payload::<HoldoutGrantConsumedPayload>(&event.payload.canonical_bytes)?
                .holdout_grant_id,
        ),
        AuditAction::ArtifactExported => Some(
            parse_payload::<ArtifactExportedPayload>(&event.payload.canonical_bytes)?.artifact_id,
        ),
        AuditAction::HoldoutApprovalRecorded => Some(
            parse_payload::<HoldoutApprovalRecordedPayload>(&event.payload.canonical_bytes)?
                .holdout_approval_record_id,
        ),
    };
    if subject
        .as_deref()
        .is_some_and(|value| value != event.target.value)
    {
        return Err(AuditError::new(
            AuditErrorCode::ActionTargetMismatch,
            "target.value",
            "subject-bearing payload identity does not match the typed target value",
        ));
    }
    Ok(())
}

fn parse_payload<T>(bytes: &[u8]) -> Result<T, AuditError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| {
        AuditError::new(
            AuditErrorCode::NonCanonicalPayload,
            "payload.canonical_bytes",
            format!("payload is not the registered closed JSON shape: {error}"),
        )
    })
}

fn validate_sha256_field(value: &str, field: &'static str) -> Result<(), AuditError> {
    Sha256Digest::parse(value).map(|_| ()).map_err(|_| {
        AuditError::new(
            AuditErrorCode::InvalidDigest,
            field,
            "value must use sha256: and 64 lowercase hexadecimal digits",
        )
    })
}

fn validate_closed_enum(
    value: &str,
    allowed: &[&str],
    field: &'static str,
) -> Result<(), AuditError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(AuditError::new(
            AuditErrorCode::InvalidEnum,
            field,
            "value is not registered in the closed audit payload enum",
        ))
    }
}

fn validate_timestamp_field(value: &str, field: &'static str) -> Result<(), AuditError> {
    validate_timestamp(value).map_err(|_| {
        AuditError::new(
            AuditErrorCode::InvalidTimestamp,
            field,
            "timestamp must be a valid UTC instant with exactly nine fractional digits",
        )
    })
}

fn validate_holdout_approval_records(
    records: &[HoldoutGrantApprovalRecord],
) -> Result<(), AuditError> {
    if records.is_empty() || records.len() > MAX_HOLDOUT_APPROVAL_RECORDS {
        return Err(AuditError::new(
            AuditErrorCode::NonCanonicalPayload,
            "payload.approval_records",
            "approval_records must contain 1..=8 entries",
        ));
    }

    let mut record_ids = HashSet::with_capacity(records.len());
    let mut record_digests = HashSet::with_capacity(records.len());
    let mut actor_ids = HashSet::with_capacity(records.len());
    let mut previous_actor_id: Option<&str> = None;
    for record in records {
        validate_domain_id(
            &record.holdout_approval_record_id,
            "payload.approval_records.holdout_approval_record_id",
        )?;
        validate_sha256_field(
            &record.approval_record_sha256,
            "payload.approval_records.approval_record_sha256",
        )?;
        validate_domain_id(
            &record.approved_by_actor_id,
            "payload.approval_records.approved_by_actor_id",
        )?;

        if !record_ids.insert(record.holdout_approval_record_id.as_str())
            || !record_digests.insert(record.approval_record_sha256.as_str())
            || !actor_ids.insert(record.approved_by_actor_id.as_str())
        {
            return Err(AuditError::new(
                AuditErrorCode::NonCanonicalPayload,
                "payload.approval_records",
                "approval record IDs, digests, and actor IDs must each be unique",
            ));
        }
        if previous_actor_id
            .is_some_and(|previous| previous >= record.approved_by_actor_id.as_str())
        {
            return Err(AuditError::new(
                AuditErrorCode::NonCanonicalPayload,
                "payload.approval_records",
                "approval_records must be in strictly increasing ASCII actor ID order",
            ));
        }
        previous_actor_id = Some(&record.approved_by_actor_id);
    }
    Ok(())
}

fn write_holdout_grant_issued_payload(payload: &HoldoutGrantIssuedPayload) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(b"{\"holdout_grant_id\":");
    write_json_string(&mut output, &payload.holdout_grant_id);
    output.extend_from_slice(b",\"holdout_period_id\":");
    write_json_string(&mut output, &payload.holdout_period_id);
    output.extend_from_slice(b",\"freeze_manifest_sha256\":");
    write_json_string(&mut output, &payload.freeze_manifest_sha256);
    output.extend_from_slice(b",\"holdout_evaluation_plan_id\":");
    write_json_string(&mut output, &payload.holdout_evaluation_plan_id);
    output.extend_from_slice(b",\"approval_records\":[");
    for (index, record) in payload.approval_records.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        output.extend_from_slice(b"{\"holdout_approval_record_id\":");
        write_json_string(&mut output, &record.holdout_approval_record_id);
        output.extend_from_slice(b",\"approval_record_sha256\":");
        write_json_string(&mut output, &record.approval_record_sha256);
        output.extend_from_slice(b",\"approved_by_actor_id\":");
        write_json_string(&mut output, &record.approved_by_actor_id);
        output.push(b'}');
    }
    output.extend_from_slice(b"],\"capability_class\":");
    write_json_string(&mut output, &payload.capability_class);
    output.extend_from_slice(b",\"authorization_decision\":");
    write_json_string(&mut output, &payload.authorization_decision);
    output.push(b'}');
    output
}

fn write_payload_fields(fields: &[(&str, &str)]) -> Vec<u8> {
    let mut output = Vec::new();
    output.push(b'{');
    for (index, (name, value)) in fields.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        output.push(b'"');
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(b"\":");
        write_json_string(&mut output, value);
    }
    output.push(b'}');
    output
}

fn write_json_string(output: &mut Vec<u8>, value: &str) {
    output.push(b'"');
    for character in value.chars() {
        match character {
            '"' => output.extend_from_slice(b"\\\""),
            '\\' => output.extend_from_slice(b"\\\\"),
            '\u{08}' => output.extend_from_slice(b"\\b"),
            '\t' => output.extend_from_slice(b"\\t"),
            '\n' => output.extend_from_slice(b"\\n"),
            '\u{0c}' => output.extend_from_slice(b"\\f"),
            '\r' => output.extend_from_slice(b"\\r"),
            '\u{00}'..='\u{1f}' => {
                let value = u32::from(character);
                output.extend_from_slice(b"\\u00");
                output.push(hex_nibble(((value >> 4) & 0x0f) as u8));
                output.push(hex_nibble((value & 0x0f) as u8));
            }
            _ => {
                let mut buffer = [0_u8; 4];
                output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    output.push(b'"');
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'a' + (value - 10),
    }
}
