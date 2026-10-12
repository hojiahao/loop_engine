use std::fs;
use std::os::unix::fs::PermissionsExt;

use clap::Parser;
use loop_protocol::wire::discovery::v1 as discovery;
use serde_json::json;

use super::*;

fn cli(args: &[&str]) -> Result<crate::Cli, clap::Error> {
    crate::Cli::try_parse_from(
        ["loopctl", "run", "--config", "/tmp/operator.json"]
            .into_iter()
            .chain(args.iter().copied()),
    )
}

fn plan() -> v1::PolicyReference {
    v1::PolicyReference {
        policy_id: Some(v1::PolicyId {
            value: "policy.research-run".into(),
        }),
        revision: "1".into(),
        sha256: Some(v1::Sha256Digest { value: vec![7; 32] }),
    }
}

fn money(amount: &str) -> v1::Money {
    v1::Money {
        amount: Some(v1::ExactDecimal {
            value: amount.into(),
        }),
        currency_code: "USD".into(),
    }
}

fn time(offset: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: 1_800_000_000 + offset,
        nanos: 0,
    }
}

fn view() -> wire::RunView {
    wire::RunView {
        run_id: Some(v1::RunId {
            value: "run.fixture".into(),
        }),
        status: wire::RunStatus::Active as i32,
        revision: 1,
        maximum_rounds: 2,
        completed_rounds: 0,
        current_job: Some(discovery::DiscoveryJobHandle {
            job_id: Some(v1::JobId {
                value: "job.child".into(),
            }),
            status: discovery::DiscoveryJobStatus::Queued as i32,
            revision: 1,
            submitted_at: Some(time(0)),
            updated_at: Some(time(0)),
        }),
        budget: Some(wire::RunBudget {
            maximum_steps: 2,
            maximum_input_tokens: 2_000,
            maximum_output_tokens: 200,
            maximum_cost: Some(money("1")),
            maximum_wall_time: Some(prost_types::Duration {
                seconds: 60,
                nanos: 0,
            }),
        }),
        reserved_steps: 1,
        reserved_input_tokens: 1_000,
        reserved_output_tokens: 100,
        reserved_cost: Some(money("0.5")),
        submitted_at: Some(time(0)),
        updated_at: Some(time(1)),
        deadline: Some(time(60)),
        plan_verified: true,
    }
}

#[test]
fn requires_start_key() {
    assert!(cli(&["start", "--plan", "/tmp/plan.binpb"]).is_err());
}

#[test]
fn requires_step_revision() {
    assert!(cli(&["step", "--run", "run.fixture", "--key", "key.step"]).is_err());
}

#[test]
fn rejects_zero_revision() {
    assert!(
        cli(&[
            "step",
            "--run",
            "run.fixture",
            "--revision",
            "0",
            "--key",
            "key.step"
        ])
        .is_err()
    );
}

#[test]
fn preserves_revision_input() {
    let crate::Command::Run(arguments) = cli(&[
        "step",
        "--run",
        "run.fixture",
        "--revision",
        "9007199254740993",
        "--key",
        "key.step",
    ])
    .unwrap()
    .command
    else {
        panic!("expected run command")
    };
    assert!(matches!(
        arguments.command,
        Command::Step {
            revision: 9_007_199_254_740_993,
            ..
        }
    ));
}

#[test]
fn bounds_command_deadline() {
    for value in ["0", "121"] {
        assert!(cli(&["--timeout-seconds", value, "status", "--run", "run.fixture"]).is_err());
    }
}

#[test]
fn accepts_canonical_plan() {
    let plan = plan();
    assert_eq!(decode_plan(&plan.encode_to_vec()), Ok(plan));
}

#[test]
fn rejects_unknown_wire() {
    let mut bytes = plan().encode_to_vec();
    bytes.extend_from_slice(&[0x20, 1]);
    assert_eq!(decode_plan(&bytes), Err(Failure::Input));
}

#[test]
fn rejects_duplicate_wire() {
    let mut bytes = plan().encode_to_vec();
    bytes.extend_from_slice(&[0x12, 1, b'1']);
    assert_eq!(decode_plan(&bytes), Err(Failure::Input));
}

#[test]
fn rejects_invalid_plan() {
    for revision in ["", "0", "01", "-1", "18446744073709551616"] {
        let mut plan = plan();
        plan.revision = revision.into();
        assert_eq!(decode_plan(&plan.encode_to_vec()), Err(Failure::Input));
    }
    let mut plan = plan();
    plan.sha256.as_mut().unwrap().value.pop();
    assert_eq!(decode_plan(&plan.encode_to_vec()), Err(Failure::Input));
}

#[test]
fn requires_private_plan() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.binpb");
    fs::write(&path, plan().encode_to_vec()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(read_plan(&path), Err(Failure::Input));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(read_plan(&path), Ok(plan()));
}

#[test]
fn separates_command_namespace() {
    let actor = v1::Actor::default();
    let first = context_with(
        actor.clone(),
        "key.step".into(),
        b"loop.run-cli.command/v1\0",
    )
    .unwrap();
    let retry = context_with(
        actor.clone(),
        "key.step".into(),
        b"loop.run-cli.command/v1\0",
    )
    .unwrap();
    let discovery =
        context_with(actor, "key.step".into(), b"loop.discovery-cli.command/v1\0").unwrap();
    assert_eq!(first.correlation_id, retry.correlation_id);
    assert_eq!(first.causation_id, retry.causation_id);
    assert_ne!(first.request_id, retry.request_id);
    assert_ne!(first.correlation_id, discovery.correlation_id);
}

#[test]
fn preserves_exact_budget() {
    let mut view = view();
    view.revision = 9_007_199_254_740_993;
    view.reserved_cost = Some(money("0.000000001"));
    let (document, code) = render("status", Some(view)).unwrap();
    assert_eq!(code, 0);
    assert_eq!(document["schema"], SCHEMA);
    assert_eq!(document["run"]["revision"], "9007199254740993");
    assert_eq!(document["run"]["reserved_cost"]["amount"], "0.000000001");
    assert_eq!(document["run"]["reserved_steps"], "1");
    assert_eq!(document["run"]["budget"]["maximum_input_tokens"], "2000");
    assert_eq!(
        document["run"]["budget"]["maximum_wall_time"]["seconds"],
        "60"
    );
}

#[test]
fn preserves_large_tokens() {
    let mut view = view();
    view.budget.as_mut().unwrap().maximum_input_tokens = i64::MAX as u64;
    view.budget.as_mut().unwrap().maximum_output_tokens = i64::MAX as u64;
    view.reserved_input_tokens = 9_007_199_254_740_993;
    view.reserved_output_tokens = i64::MAX as u64;
    let (document, _) = render("status", Some(view)).unwrap();
    assert_eq!(document["run"]["reserved_input_tokens"], "9007199254740993");
    assert_eq!(
        document["run"]["reserved_output_tokens"],
        "9223372036854775807"
    );
    assert_eq!(
        document["run"]["budget"]["maximum_input_tokens"],
        "9223372036854775807"
    );
}

#[test]
fn rejects_excess_reservation() {
    let mut view = view();
    view.reserved_cost = Some(money("1.000000001"));
    assert_eq!(render("status", Some(view)), Err(Failure::Protocol));
}

#[test]
fn displays_stop_status() {
    let mut view = view();
    view.plan_verified = false;
    let (document, code) = render("status", Some(view)).unwrap();
    assert_eq!(code, 0);
    assert_eq!(document["run"]["plan_verified"], false);
    assert!(document["run"].get("candidate").is_none());
}

#[test]
fn distinguishes_terminal_status() {
    for status in [
        wire::RunStatus::BudgetExhausted,
        wire::RunStatus::InfrastructureFailed,
        wire::RunStatus::DeadlineExceeded,
    ] {
        let mut view = view();
        view.status = status as i32;
        view.updated_at = Some(time(60));
        assert_eq!(render("status", Some(view)).unwrap().1, 7);
    }
    let mut view = view();
    view.status = wire::RunStatus::Completed as i32;
    view.completed_rounds = 2;
    view.current_job.as_mut().unwrap().status = discovery::DiscoveryJobStatus::Succeeded as i32;
    assert_eq!(render("status", Some(view)).unwrap().1, 0);
}

#[test]
fn rejects_invalid_projection() {
    assert_eq!(render("status", None), Err(Failure::Protocol));
    let mut view = view();
    view.status = 999;
    assert_eq!(render("status", Some(view)), Err(Failure::Protocol));
}

#[test]
fn redacts_run_errors() {
    let error = Failure::from_status(tonic::Status::permission_denied(
        "secret-token /private/path",
    ));
    assert_eq!(
        error.envelope(SCHEMA),
        json!({"schema":SCHEMA,"error":{"category":"authorization"}})
    );
    assert_eq!(Failure::Interrupted.envelope(SCHEMA)["schema"], SCHEMA);
    assert_eq!(Failure::Interrupted.exit_code(), 130);
}
