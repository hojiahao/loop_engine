use super::*;

fn time() -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: 1_800_000_000,
        nanos: 123_456_789,
    }
}

fn source(status: wire::DiscoveryJobStatus) -> wire::DiscoveryJobHandle {
    wire::DiscoveryJobHandle {
        job_id: Some(v1::JobId {
            value: "job.fixture".into(),
        }),
        status: status as i32,
        revision: 9_007_199_254_740_993,
        submitted_at: Some(time()),
        updated_at: Some(time()),
    }
}

fn view() -> wire::DiscoveryStepView {
    wire::DiscoveryStepView {
        job: Some(source(wire::DiscoveryJobStatus::Running)),
        state: wire::DiscoveryStepState::Dispatched as i32,
        candidate: None,
        reserved_cost: Some(v1::Money {
            amount: Some(v1::ExactDecimal {
                value: "0.000000001".into(),
            }),
            currency_code: "USD".into(),
        }),
        reserved_input_tokens: 9_007_199_254_740_993,
        reserved_output_tokens: u64::MAX,
        plan_verified: true,
    }
}

fn ast(bytes: &[u8]) -> wire::DiscoveryCandidate {
    let mut hash = Sha256::new();
    hash.update(b"loop.factor-ast/v1\0");
    hash.update(bytes);
    wire::DiscoveryCandidate {
        expression_id: Some(v1::FactorExpressionId {
            value: format!("sha256:{:x}", hash.finalize()),
        }),
        canonicalization_profile: "loop.factor-ast/v1".into(),
        canonical_json: bytes.to_vec(),
    }
}

fn event(sequence: u64) -> wire::DiscoveryEvent {
    wire::DiscoveryEvent {
        sequence,
        occurred_at: Some(time()),
        operation: wire::DiscoveryOperation::Start as i32,
    }
}

#[test]
fn preserves_large_integers() {
    let (output, code) = step("status", Some(view())).unwrap();
    assert_eq!(code, 0);
    assert_eq!(output["job"]["revision"], "9007199254740993");
    assert_eq!(output["step"]["reserved_input_tokens"], "9007199254740993");
    assert_eq!(
        output["step"]["reserved_output_tokens"],
        "18446744073709551615"
    );
    assert_eq!(output["job"]["submitted_at"]["seconds"], "1800000000");
    assert_eq!(output["job"]["submitted_at"]["nanos"], "123456789");
}

#[test]
fn preserves_exact_money() {
    let (output, _) = step("status", Some(view())).unwrap();
    assert_eq!(output["step"]["reserved_cost"]["amount"], "0.000000001");
}

#[test]
fn distinguishes_terminal_failures() {
    for status in [
        wire::DiscoveryJobStatus::FactorRejected,
        wire::DiscoveryJobStatus::InfrastructureFailed,
        wire::DiscoveryJobStatus::Cancelled,
        wire::DiscoveryJobStatus::BudgetExhausted,
    ] {
        assert_eq!(job("status", Some(source(status))).unwrap().1, 7);
    }
}

#[test]
fn rejects_unknown_status() {
    let mut job = source(wire::DiscoveryJobStatus::Queued);
    job.status = 999;
    assert_eq!(handle(job), Err(Failure::Protocol));
}

#[test]
fn rejects_unknown_step() {
    let mut value = view();
    value.state = 999;
    assert_eq!(step("status", Some(value)), Err(Failure::Protocol));
}

#[test]
fn rejects_negative_reservation() {
    let mut value = view();
    value
        .reserved_cost
        .as_mut()
        .unwrap()
        .amount
        .as_mut()
        .unwrap()
        .value = "-1".into();
    assert_eq!(step("status", Some(value)), Err(Failure::Protocol));
}

#[test]
fn projects_stop_metadata() {
    let mut value = view();
    value.plan_verified = false;
    let (output, _) = step("status", Some(value)).unwrap();
    assert_eq!(output["step"]["plan_verified"], false);
    assert!(output["step"].get("candidate").is_none());
}

#[test]
fn projects_queued_job() {
    let value = wire::DiscoveryStepView {
        job: Some(source(wire::DiscoveryJobStatus::Queued)),
        plan_verified: true,
        ..Default::default()
    };
    let (output, code) = step("status", Some(value)).unwrap();
    assert_eq!(code, 0);
    assert_eq!(output["step"]["state"], "unspecified");
    assert!(output["step"]["reserved_cost"].is_null());
}

#[test]
fn projects_unreserved_pause() {
    let value = wire::DiscoveryStepView {
        job: Some(source(wire::DiscoveryJobStatus::Paused)),
        ..Default::default()
    };
    let (output, code) = step("status", Some(value)).unwrap();
    assert_eq!(code, 0);
    assert_eq!(output["step"]["plan_verified"], false);
    assert!(output["step"]["reserved_cost"].is_null());
}

#[test]
fn rejects_unverified_candidate() {
    let mut value = view();
    value.job.as_mut().unwrap().status = wire::DiscoveryJobStatus::Succeeded as i32;
    value.state = wire::DiscoveryStepState::Completed as i32;
    value.plan_verified = false;
    value.candidate = Some(ast(br#"{"node":"field","field":"close"}"#));
    assert_eq!(step("status", Some(value)), Err(Failure::Protocol));
}

#[test]
fn rejects_nonterminal_candidate() {
    let mut value = view();
    value.candidate = Some(ast(br#"{"node":"field","field":"close"}"#));
    assert_eq!(step("status", Some(value)), Err(Failure::Protocol));
}

#[test]
fn projects_canonical_candidate() {
    let mut value = view();
    value.job.as_mut().unwrap().status = wire::DiscoveryJobStatus::Succeeded as i32;
    value.state = wire::DiscoveryStepState::Completed as i32;
    value.candidate = Some(ast(br#"{"node":"field","field":"close"}"#));
    let (output, code) = step("execute", Some(value)).unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        output["step"]["candidate"]["canonical_json"],
        r#"{"node":"field","field":"close"}"#
    );
}

#[test]
fn rejects_reconcile_candidate() {
    let mut value = view();
    value.job.as_mut().unwrap().status = wire::DiscoveryJobStatus::Succeeded as i32;
    value.state = wire::DiscoveryStepState::Completed as i32;
    value.candidate = Some(ast(br#"{"node":"field","field":"close"}"#));
    assert_eq!(step("reconcile", Some(value)), Err(Failure::Protocol));
}

#[test]
fn rejects_candidate_extras() {
    let value = ast(br#"{"node":"field","field":"close","secret":"sensitive"}"#);
    assert_eq!(candidate(value), Err(Failure::Protocol));
}

#[test]
fn rejects_candidate_identity() {
    let mut value = ast(br#"{"node":"field","field":"close"}"#);
    value.expression_id.as_mut().unwrap().value = format!("sha256:{}", "0".repeat(64));
    assert_eq!(candidate(value), Err(Failure::Protocol));
}

#[test]
fn rejects_candidate_encoding() {
    assert_eq!(
        candidate(ast(br#"{"field":"close","node":"field"}"#)),
        Err(Failure::Protocol)
    );
}

#[test]
fn rejects_event_order() {
    let response = wire::ListDiscoveryEventsResponse {
        events: vec![event(3), event(2)],
        next_after_sequence: 2,
        has_more: false,
    };
    assert_eq!(
        events("job.fixture", 0, 100, response),
        Err(Failure::Protocol)
    );
}

#[test]
fn rejects_unknown_operation() {
    let mut value = event(1);
    value.operation = 999;
    let response = wire::ListDiscoveryEventsResponse {
        events: vec![value],
        next_after_sequence: 1,
        has_more: false,
    };
    assert_eq!(
        events("job.fixture", 0, 100, response),
        Err(Failure::Protocol)
    );
}

#[test]
fn rejects_cursor_mismatch() {
    let response = wire::ListDiscoveryEventsResponse {
        events: vec![event(1)],
        next_after_sequence: 2,
        has_more: false,
    };
    assert_eq!(
        events("job.fixture", 0, 100, response),
        Err(Failure::Protocol)
    );
}

#[test]
fn preserves_event_sequence() {
    let sequence = 9_007_199_254_740_993;
    let response = wire::ListDiscoveryEventsResponse {
        events: vec![event(sequence)],
        next_after_sequence: sequence,
        has_more: false,
    };
    let (output, code) = events("job.fixture", 0, 100, response).unwrap();
    assert_eq!(code, 0);
    assert_eq!(output["events"][0]["sequence"], "9007199254740993");
    assert_eq!(output["next_after_sequence"], "9007199254740993");
}

#[test]
fn redacts_server_errors() {
    let error = Failure::from_status(tonic::Status::permission_denied(
        "SECRET /private/config https://user:password@service",
    ));
    assert_eq!(error.exit_code(), 3);
    assert_eq!(
        error.envelope(),
        json!({"schema":"loop.discovery-cli/v1","error":{"category":"authorization"}})
    );
}

#[test]
fn distinguishes_interruption() {
    assert_eq!(Failure::Interrupted.exit_code(), 130);
    assert_eq!(
        Failure::Interrupted.envelope()["error"]["category"],
        "interrupted"
    );
    assert_eq!(Failure::RemoteCancelled.exit_code(), 6);
}
