use serde::Deserialize;

use super::*;

const VECTORS: &str =
    include_str!("../../../../fixtures/contracts/audit/v1/canonical_chain_vectors.json");
const ACTION_BINDING_VECTORS: &str =
    include_str!("../../../../fixtures/contracts/audit/v1/action_binding_vectors.json");

#[derive(Debug, Deserialize)]
struct Fixture {
    schema: String,
    accepted_chain: Vec<AcceptedVector>,
    action_values: Vec<String>,
    target_vectors: Vec<TargetVector>,
    boundary_vectors: BoundaryVectors,
    negative_vectors: Vec<NegativeVector>,
}

#[derive(Debug, Deserialize)]
struct BoundaryVectors {
    max_domain_id_bytes: String,
    max_schema_text_bytes: String,
    max_payload_bytes: String,
    max_schema_version: String,
    max_sequence: String,
}

#[derive(Debug, Deserialize)]
struct AcceptedVector {
    name: String,
    payload: PayloadVector,
    event: EventVector,
    canonical_event_utf8: String,
    event_sha256: String,
}

#[derive(Debug, Deserialize)]
struct PayloadVector {
    schema_name: String,
    schema_version: String,
    canonical_utf8: String,
    payload_sha256: String,
}

#[derive(Debug, Deserialize)]
struct EventVector {
    audit_ledger_id: String,
    sequence: String,
    previous_event_sha256: String,
    audit_event_id: String,
    occurred_at: String,
    correlation_id: String,
    causation_id: String,
    actor: ActorVector,
    action: String,
    target: TargetVector,
}

#[derive(Debug, Deserialize)]
struct ActorVector {
    actor_id: String,
    kind: String,
    display_name: String,
    authenticated_subject: String,
}

#[derive(Debug, Deserialize)]
struct TargetVector {
    kind: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct NegativeVector {
    name: String,
    mutation: String,
    expected_code: String,
}

#[derive(Debug, Deserialize)]
struct ActionBindingFixture {
    schema: String,
    event_envelope: ActionEventEnvelope,
    accepted: Vec<ActionBindingVector>,
    malformed_payloads: Vec<MalformedPayloadVector>,
}

#[derive(Debug, Deserialize)]
struct ActionEventEnvelope {
    audit_ledger_id: String,
    sequence: String,
    previous_event_sha256: String,
    audit_event_id: String,
    occurred_at: String,
    correlation_id: String,
    causation_id: String,
    actor: ActorVector,
}

#[derive(Debug, Deserialize)]
struct ActionBindingVector {
    name: String,
    action: String,
    payload_schema: String,
    canonical_payload: String,
    target: TargetVector,
    forbidden_target: TargetVector,
    mismatched_target_value: Option<String>,
    payload_sha256: String,
    canonical_event_utf8: String,
    event_sha256: String,
}

#[derive(Debug, Deserialize)]
struct MalformedPayloadVector {
    name: String,
    payload_schema: String,
    canonical_payload: String,
    expected_code: String,
}

#[test]
fn shared_chain_vectors_are_byte_and_digest_exact() {
    let fixture = fixture();
    assert_eq!(fixture.schema, "loop.audit-conformance/v1");
    let chain = chain(&fixture);
    for (event, vector) in chain.iter().zip(&fixture.accepted_chain) {
        assert_eq!(
            event.payload.canonical_bytes,
            vector.payload.canonical_utf8.as_bytes(),
            "{} payload bytes",
            vector.name
        );
        assert_eq!(
            event.payload.payload_sha256.to_string(),
            vector.payload.payload_sha256,
            "{} payload digest",
            vector.name
        );
        assert_eq!(
            canonical_audit_event_bytes(event).unwrap(),
            vector.canonical_event_utf8.as_bytes(),
            "{} event bytes",
            vector.name
        );
        assert_eq!(
            audit_event_sha256(event).unwrap().to_string(),
            vector.event_sha256,
            "{} event digest",
            vector.name
        );
    }
    verify_audit_chain(&chain).unwrap();
}

#[test]
fn every_action_and_typed_target_enum_spelling_is_closed() {
    let fixture = fixture();
    for action in &fixture.action_values {
        AuditAction::try_from(action.as_str()).unwrap();
    }
    assert_eq!(
        AuditAction::try_from("unknown").unwrap_err().code(),
        AuditErrorCode::InvalidEnum
    );

    for target in &fixture.target_vectors {
        let kind = AuditTargetKind::try_from(target.kind.as_str()).unwrap();
        validate_target(&AuditTarget {
            kind,
            value: target.value.clone(),
        })
        .unwrap();
    }
    assert_eq!(
        AuditTargetKind::try_from("unknown").unwrap_err().code(),
        AuditErrorCode::InvalidTarget
    );
}

#[test]
fn shared_action_registry_binds_schema_target_and_subject_before_hashing() {
    let fixture = action_binding_fixture();
    assert_eq!(fixture.schema, "loop.audit-action-binding/v1");
    assert_eq!(fixture.accepted.len(), 11);
    let malformed_names = fixture
        .malformed_payloads
        .iter()
        .map(|vector| vector.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    for required in [
        "holdout_approval_records_unsorted",
        "holdout_approval_record_id_duplicate",
        "holdout_approval_record_digest_duplicate",
        "holdout_approval_actor_duplicate",
        "invalid_holdout_approval_record_digest",
        "invalid_holdout_approval_actor_id",
        "unknown_holdout_capability_class",
        "unknown_holdout_authorization_decision",
    ] {
        assert!(
            malformed_names.contains(required),
            "missing {required} vector"
        );
    }
    for (index, vector) in fixture.accepted.iter().enumerate() {
        let event = action_event(&fixture.event_envelope, vector);
        assert_eq!(
            event.payload.payload_sha256.to_string(),
            vector.payload_sha256,
            "{} payload digest",
            vector.name
        );
        assert_eq!(
            canonical_audit_event_bytes(&event).unwrap(),
            vector.canonical_event_utf8.as_bytes(),
            "{} event bytes",
            vector.name
        );
        assert_eq!(
            audit_event_sha256(&event).unwrap().to_string(),
            vector.event_sha256,
            "{} event digest",
            vector.name
        );
        let mut sealed = event.clone();
        sealed.event_sha256 = Sha256Digest::parse(&vector.event_sha256).unwrap();
        verify_audit_event(&sealed).unwrap();

        let wrong_action = &fixture.accepted[(index + 1) % fixture.accepted.len()];
        let mut wrong_schema = event.clone();
        wrong_schema.action = AuditAction::try_from(wrong_action.action.as_str()).unwrap();
        wrong_schema.target = AuditTarget {
            kind: AuditTargetKind::try_from(wrong_action.target.kind.as_str()).unwrap(),
            value: wrong_action.target.value.clone(),
        };
        assert_eq!(
            audit_event_sha256(&wrong_schema).unwrap_err().code(),
            AuditErrorCode::ActionPayloadMismatch,
            "{} wrong action/schema",
            vector.name
        );

        let mut forbidden_target = event.clone();
        forbidden_target.target = AuditTarget {
            kind: AuditTargetKind::try_from(vector.forbidden_target.kind.as_str()).unwrap(),
            value: vector.forbidden_target.value.clone(),
        };
        assert_eq!(
            audit_event_sha256(&forbidden_target).unwrap_err().code(),
            AuditErrorCode::ActionTargetMismatch,
            "{} forbidden target",
            vector.name
        );

        if let Some(mismatched_target_value) = &vector.mismatched_target_value {
            let mut mismatched_subject = event.clone();
            mismatched_subject.target.value = mismatched_target_value.clone();
            assert_eq!(
                audit_event_sha256(&mismatched_subject).unwrap_err().code(),
                AuditErrorCode::ActionTargetMismatch,
                "{} mismatched subject",
                vector.name
            );
        }
    }

    for vector in &fixture.malformed_payloads {
        assert_eq!(
            canonicalize_audit_payload(
                &vector.payload_schema,
                1,
                vector.canonical_payload.as_bytes()
            )
            .unwrap_err()
            .code()
            .as_str(),
            vector.expected_code,
            "{}",
            vector.name
        );
    }
}

#[test]
fn hostile_deep_payload_fails_closed_without_panicking() {
    let depth = 512;
    let mut payload = "[".repeat(depth);
    payload.push_str("{}\n");
    payload.push_str(&"]".repeat(depth));
    assert_eq!(
        canonicalize_audit_payload("loop.audit.command_accepted", 1, payload.as_bytes())
            .unwrap_err()
            .code(),
        AuditErrorCode::NonCanonicalPayload
    );
}

#[test]
fn shared_tamper_reorder_genesis_and_cross_ledger_vectors_fail_closed() {
    let fixture = fixture();
    for negative in &fixture.negative_vectors {
        let mut events = chain(&fixture);
        match negative.mutation.as_str() {
            "payload_tamper" => {
                events[0].payload.canonical_bytes = events[0]
                    .payload
                    .canonical_bytes
                    .windows(2)
                    .position(|window| window == b"ok")
                    .map(|index| {
                        let mut bytes = events[0].payload.canonical_bytes.clone();
                        bytes[index..index + 2].copy_from_slice(b"no");
                        bytes
                    })
                    .expect("fixture payload contains ok");
            }
            "event_tamper" => events[0].actor.display_name.push('!'),
            "event_reorder" => events.swap(0, 1),
            "previous_digest_tamper" => {
                events[1].previous_event_sha256 = Sha256Digest::from_bytes([1; 32]);
                events[1].event_sha256 = audit_event_sha256(&events[1]).unwrap();
            }
            "cross_ledger_replay" => events[1].audit_ledger_id = "ledger.secondary".into(),
            "sequence_gap" => {
                events[1].sequence = 3;
                events[1].event_sha256 = audit_event_sha256(&events[1]).unwrap();
            }
            "invalid_genesis" => {
                events[0].previous_event_sha256 = Sha256Digest::from_bytes([2; 32]);
                events[0].event_sha256 = audit_event_sha256(&events[0]).unwrap();
            }
            "invalid_timestamp" => {
                events[0].occurred_at = "2026-09-05T06:30:00Z".into();
            }
            "duplicate_event_id" => {
                events[1].audit_event_id = events[0].audit_event_id.clone();
                events[1].event_sha256 = audit_event_sha256(&events[1]).unwrap();
            }
            "schema_mutation" => {
                events[0].payload.schema_name = "loop.audit.unknown".into();
            }
            "schema_version_mutation" => {
                events[0].payload.schema_version = 2;
            }
            unknown => panic!("unknown negative mutation {unknown}"),
        }
        let error = verify_audit_chain(&events).unwrap_err();
        assert_eq!(
            error.code().as_str(),
            negative.expected_code,
            "{}",
            negative.name
        );
    }
}

#[test]
fn shared_boundary_values_are_accepted_and_overruns_are_rejected() {
    let fixture = fixture();
    let boundaries = &fixture.boundary_vectors;
    let max_id_bytes: usize = boundaries.max_domain_id_bytes.parse().unwrap();
    let max_text_bytes: usize = boundaries.max_schema_text_bytes.parse().unwrap();
    let max_payload_bytes: usize = boundaries.max_payload_bytes.parse().unwrap();
    let max_schema_version: u32 = boundaries.max_schema_version.parse().unwrap();
    let max_sequence: u64 = boundaries.max_sequence.parse().unwrap();

    let mut event = chain(&fixture).remove(0);
    event.audit_ledger_id = "a".repeat(max_id_bytes);
    event.sequence = max_sequence;
    event.actor.display_name = "x".repeat(max_text_bytes);
    event.event_sha256 = audit_event_sha256(&event).unwrap();
    verify_audit_event(&event).unwrap();

    let summary = "x".repeat(max_text_bytes);
    let payload = format!(
        "{{\"command\":\"research.run\",\"request_id\":\"request.1\",\"summary\":\"{summary}\"}}"
    );
    canonicalize_audit_payload("loop.audit.command_accepted", 1, payload.as_bytes()).unwrap();
    let oversized_text = payload.replace(&summary, &"x".repeat(max_text_bytes + 1));
    assert_eq!(
        canonicalize_audit_payload("loop.audit.command_accepted", 1, oversized_text.as_bytes())
            .unwrap_err()
            .code(),
        AuditErrorCode::InvalidText
    );
    assert_eq!(
        canonicalize_audit_payload(
            "loop.audit.command_accepted",
            1,
            &vec![b'x'; max_payload_bytes + 1]
        )
        .unwrap_err()
        .code(),
        AuditErrorCode::SizeLimit
    );
    assert!(audit_payload_sha256("loop.audit.command_accepted", max_schema_version, b"").is_ok());
    assert_eq!(
        validate_domain_id(&"a".repeat(max_id_bytes + 1), "test")
            .unwrap_err()
            .code(),
        AuditErrorCode::InvalidIdentifier
    );
}

#[test]
fn payload_registry_rejects_noncanonical_and_unknown_documents() {
    for value in [
        br#" {"command":"research.run","request_id":"request.1","summary":"ok"}"#.as_slice(),
        br#"{"request_id":"request.1","command":"research.run","summary":"ok"}"#.as_slice(),
        br#"{"command":"research.run","request_id":"request.1","summary":"ok","extra":"x"}"#
            .as_slice(),
        br#"{"command":"research.run","command":"research.run","request_id":"request.1","summary":"ok"}"#
            .as_slice(),
    ] {
        assert_eq!(
            canonicalize_audit_payload("loop.audit.command_accepted", 1, value)
                .unwrap_err()
                .code(),
            AuditErrorCode::NonCanonicalPayload
        );
    }
    assert_eq!(
        canonicalize_audit_payload("loop.audit.unknown", 1, b"{}")
            .unwrap_err()
            .code(),
        AuditErrorCode::UnsupportedSchema
    );
}

#[test]
fn holdout_grant_issued_binds_canonical_approval_evidence() {
    let first = approval_record_json("approval.01", '1', "actor.risk");
    let second = approval_record_json("approval.02", '2', "actor.security");
    let canonical = holdout_grant_issued_json(
        &[first.clone(), second.clone()],
        "holdout_evaluation",
        "authorized",
    );
    let payload =
        canonicalize_audit_payload("loop.audit.holdout_grant_issued", 1, canonical.as_bytes())
            .unwrap();
    assert_eq!(payload.canonical_bytes, canonical.as_bytes());

    let too_many = (0_u8..9)
        .map(|index| {
            approval_record_json(
                &format!("approval.{index:02}"),
                char::from(b'0' + index),
                &format!("actor.{index:02}"),
            )
        })
        .collect::<Vec<_>>();
    let invalid_record_sets = [
        Vec::new(),
        too_many,
        vec![second.clone(), first.clone()],
        vec![
            approval_record_json("approval.01", '1', "actor.risk"),
            approval_record_json("approval.01", '2', "actor.security"),
        ],
        vec![
            approval_record_json("approval.01", '1', "actor.risk"),
            approval_record_json("approval.02", '1', "actor.security"),
        ],
        vec![
            approval_record_json("approval.01", '1', "actor.risk"),
            approval_record_json("approval.02", '2', "actor.risk"),
        ],
    ];
    for records in invalid_record_sets {
        let submitted = holdout_grant_issued_json(&records, "holdout_evaluation", "authorized");
        assert_eq!(
            canonicalize_audit_payload("loop.audit.holdout_grant_issued", 1, submitted.as_bytes())
                .unwrap_err()
                .code(),
            AuditErrorCode::NonCanonicalPayload
        );
    }

    let reordered_record = format!(
        "{{\"approval_record_sha256\":\"sha256:{}\",\"holdout_approval_record_id\":\"approval.01\",\"approved_by_actor_id\":\"actor.risk\"}}",
        "1".repeat(64)
    );
    let submitted =
        holdout_grant_issued_json(&[reordered_record], "holdout_evaluation", "authorized");
    assert_eq!(
        canonicalize_audit_payload("loop.audit.holdout_grant_issued", 1, submitted.as_bytes())
            .unwrap_err()
            .code(),
        AuditErrorCode::NonCanonicalPayload
    );
}

#[test]
fn holdout_grant_audit_authorization_values_are_closed() {
    let record = approval_record_json("approval.01", '1', "actor.risk");
    for submitted in [
        holdout_grant_issued_json(
            std::slice::from_ref(&record),
            "holdout_export",
            "authorized",
        ),
        holdout_grant_issued_json(&[record], "holdout_evaluation", "denied"),
        holdout_grant_consumed_json("holdout_export", "authorized"),
        holdout_grant_consumed_json("holdout_evaluation", "denied"),
    ] {
        let schema = if submitted.contains("approval_records") {
            "loop.audit.holdout_grant_issued"
        } else {
            "loop.audit.holdout_grant_consumed"
        };
        assert_eq!(
            canonicalize_audit_payload(schema, 1, submitted.as_bytes())
                .unwrap_err()
                .code(),
            AuditErrorCode::InvalidEnum
        );
    }

    let consumed = holdout_grant_consumed_json("holdout_evaluation", "authorized");
    assert!(
        canonicalize_audit_payload("loop.audit.holdout_grant_consumed", 1, consumed.as_bytes())
            .is_ok()
    );
}

#[test]
fn timestamp_and_digest_encodings_are_strict() {
    for timestamp in [
        "2026-02-29T00:00:00.000000000Z",
        "2026-09-05T06:30:60.000000000Z",
        "2026-09-05T06:30:00.00000000Z",
        "2026-09-05T06:30:00.000000000+00:00",
        "0000-01-01T00:00:00.000000000Z",
    ] {
        assert_eq!(
            validate_timestamp(timestamp).unwrap_err().code(),
            AuditErrorCode::InvalidTimestamp
        );
    }
    assert!(validate_timestamp("2024-02-29T23:59:59.999999999Z").is_ok());
    assert_eq!(
        Sha256Digest::parse(&format!("sha256:{}", "A".repeat(64)))
            .unwrap_err()
            .code(),
        AuditErrorCode::InvalidDigest
    );
}

fn approval_record_json(record_id: &str, digest_nibble: char, actor_id: &str) -> String {
    format!(
        "{{\"holdout_approval_record_id\":\"{record_id}\",\"approval_record_sha256\":\"sha256:{}\",\"approved_by_actor_id\":\"{actor_id}\"}}",
        digest_nibble.to_string().repeat(64)
    )
}

fn holdout_grant_issued_json(
    approval_records: &[String],
    capability_class: &str,
    authorization_decision: &str,
) -> String {
    format!(
        "{{\"holdout_grant_id\":\"grant.01\",\"holdout_period_id\":\"sha256:{}\",\"freeze_manifest_sha256\":\"sha256:{}\",\"holdout_evaluation_plan_id\":\"sha256:{}\",\"approval_records\":[{}],\"capability_class\":\"{capability_class}\",\"authorization_decision\":\"{authorization_decision}\"}}",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        approval_records.join(",")
    )
}

fn holdout_grant_consumed_json(capability_class: &str, authorization_decision: &str) -> String {
    format!(
        "{{\"holdout_grant_id\":\"grant.01\",\"holdout_period_id\":\"sha256:{}\",\"holdout_evaluation_plan_id\":\"sha256:{}\",\"job_batch_id\":\"batch.01\",\"capability_class\":\"{capability_class}\",\"authorization_decision\":\"{authorization_decision}\"}}",
        "a".repeat(64),
        "c".repeat(64),
    )
}

fn fixture() -> Fixture {
    serde_json::from_str(VECTORS).expect("shared audit fixture must be valid JSON")
}

fn action_binding_fixture() -> ActionBindingFixture {
    serde_json::from_str(ACTION_BINDING_VECTORS)
        .expect("shared audit action-binding fixture must be valid JSON")
}

fn action_event(envelope: &ActionEventEnvelope, vector: &ActionBindingVector) -> AuditEvent {
    let payload = canonicalize_audit_payload(
        &vector.payload_schema,
        1,
        vector.canonical_payload.as_bytes(),
    )
    .unwrap();
    AuditEvent {
        audit_ledger_id: envelope.audit_ledger_id.clone(),
        sequence: envelope.sequence.parse().unwrap(),
        previous_event_sha256: Sha256Digest::parse(&envelope.previous_event_sha256).unwrap(),
        audit_event_id: envelope.audit_event_id.clone(),
        occurred_at: envelope.occurred_at.clone(),
        correlation_id: envelope.correlation_id.clone(),
        causation_id: envelope.causation_id.clone(),
        actor: AuditActor {
            actor_id: envelope.actor.actor_id.clone(),
            kind: ActorKind::try_from(envelope.actor.kind.as_str()).unwrap(),
            display_name: envelope.actor.display_name.clone(),
            authenticated_subject: envelope.actor.authenticated_subject.clone(),
        },
        action: AuditAction::try_from(vector.action.as_str()).unwrap(),
        target: AuditTarget {
            kind: AuditTargetKind::try_from(vector.target.kind.as_str()).unwrap(),
            value: vector.target.value.clone(),
        },
        payload,
        event_sha256: Sha256Digest::ZERO,
    }
}

fn chain(fixture: &Fixture) -> Vec<AuditEvent> {
    fixture
        .accepted_chain
        .iter()
        .map(|vector| {
            let payload = canonicalize_audit_payload(
                &vector.payload.schema_name,
                vector.payload.schema_version.parse().unwrap(),
                vector.payload.canonical_utf8.as_bytes(),
            )
            .unwrap();
            assert_eq!(
                payload.payload_sha256.to_string(),
                vector.payload.payload_sha256
            );
            AuditEvent {
                audit_ledger_id: vector.event.audit_ledger_id.clone(),
                sequence: vector.event.sequence.parse().unwrap(),
                previous_event_sha256: Sha256Digest::parse(&vector.event.previous_event_sha256)
                    .unwrap(),
                audit_event_id: vector.event.audit_event_id.clone(),
                occurred_at: vector.event.occurred_at.clone(),
                correlation_id: vector.event.correlation_id.clone(),
                causation_id: vector.event.causation_id.clone(),
                actor: AuditActor {
                    actor_id: vector.event.actor.actor_id.clone(),
                    kind: ActorKind::try_from(vector.event.actor.kind.as_str()).unwrap(),
                    display_name: vector.event.actor.display_name.clone(),
                    authenticated_subject: vector.event.actor.authenticated_subject.clone(),
                },
                action: AuditAction::try_from(vector.event.action.as_str()).unwrap(),
                target: AuditTarget {
                    kind: AuditTargetKind::try_from(vector.event.target.kind.as_str()).unwrap(),
                    value: vector.event.target.value.clone(),
                },
                payload,
                event_sha256: Sha256Digest::parse(&vector.event_sha256).unwrap(),
            }
        })
        .collect()
}
