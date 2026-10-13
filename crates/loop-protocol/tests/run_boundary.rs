#![cfg(feature = "runs-service")]

use std::collections::{BTreeMap, BTreeSet};

use loop_protocol::runs::{validate_view, validate_workflow};
use loop_protocol::wire::discovery::v1::DiscoveryJobStatus;
use loop_protocol::wire::research::v1::ResearchJobStatus;
use loop_protocol::wire::runs::v1::{
    RoundDisposition, RunStage, RunStatus, RunView, research_run_view::CurrentChild,
};
use prost::Message;
use prost_types::FileDescriptorSet;

fn golden() -> (RunView, Vec<u8>) {
    decode(include_str!(
        "../../../fixtures/contracts/protocol/v1/run_view_v1.json"
    ))
}

fn numerical() -> (RunView, Vec<u8>) {
    decode(include_str!(
        "../../../fixtures/contracts/protocol/v1/run_view_v2.json"
    ))
}

fn decode(source: &str) -> (RunView, Vec<u8>) {
    let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
    let wire = fixture["wire_hex"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|bytes| u8::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    (RunView::decode(wire.as_slice()).unwrap(), wire)
}

#[test]
fn numerical_projection() {
    let (view, wire) = numerical();
    validate_view(&view).unwrap();
    assert_eq!(
        view.research.as_ref().unwrap().stage,
        RunStage::Evaluation as i32
    );
    assert_eq!(view.encode_to_vec(), wire);
    validate_workflow(0).unwrap();
    validate_workflow(2).unwrap();
    assert!(validate_workflow(1).is_err());
    assert!(validate_workflow(u32::MAX).is_err());
}

#[test]
fn numerical_failures() {
    let changes: &[fn(&mut RunView)] = &[
        |view| view.current_job = golden().0.current_job,
        |view| view.revision = 8,
        |view| view.research.as_mut().unwrap().round = 0,
        |view| view.research.as_mut().unwrap().round = 1,
        |view| view.research.as_mut().unwrap().stage = 127,
        |view| view.research.as_mut().unwrap().stage = 0,
        |view| view.research.as_mut().unwrap().stage = RunStage::Discovery as i32,
        |view| view.research.as_mut().unwrap().binding_sha256 = None,
        |view| {
            view.research
                .as_mut()
                .unwrap()
                .binding_sha256
                .as_mut()
                .unwrap()
                .value
                .pop();
        },
        |view| view.research.as_mut().unwrap().current_child = None,
        |view| view.research.as_mut().unwrap().evaluation_job_id = None,
        |view| view.research.as_mut().unwrap().discovery_job_id = None,
        |view| view.research.as_mut().unwrap().expression_id = None,
        |view| {
            view.research
                .as_mut()
                .unwrap()
                .factor_spec_id
                .as_mut()
                .unwrap()
                .value
                .push('\n')
        },
        |view| view.research.as_mut().unwrap().disposition = 127,
        |view| view.research.as_mut().unwrap().disposition = RoundDisposition::Evaluated as i32,
        |view| {
            view.research.as_mut().unwrap().disposition =
                RoundDisposition::InsufficientCoverage as i32
        },
        |view| {
            let research = view.research.as_mut().unwrap();
            research.discovery_job_id = research.evaluation_job_id.clone();
        },
    ];
    for (index, mutate) in changes.iter().enumerate() {
        let (mut view, _) = numerical();
        mutate(&mut view);
        assert!(validate_view(&view).is_err(), "accepted v2 case {index}");
    }
}

#[test]
fn numerical_terminals() {
    let (mut view, _) = numerical();
    view.status = RunStatus::Completed as i32;
    view.completed_rounds = view.maximum_rounds;
    let research = view.research.as_mut().unwrap();
    research.disposition = RoundDisposition::InsufficientCoverage as i32;
    let Some(CurrentChild::Research(child)) = research.current_child.as_mut() else {
        panic!("fixture research handle");
    };
    child.status = ResearchJobStatus::Succeeded as i32;
    validate_view(&view).unwrap();
    let research = view.research.as_mut().unwrap();
    research.stage = RunStage::Portfolio as i32;
    research.disposition = RoundDisposition::Evaluated as i32;
    let Some(CurrentChild::Research(child)) = research.current_child.as_mut() else {
        panic!("fixture research handle");
    };
    child.job_id.as_mut().unwrap().value = "job.portfolio.2".into();
    validate_view(&view).unwrap();
    view.research.as_mut().unwrap().disposition = 0;
    assert!(validate_view(&view).is_err());
}

#[test]
fn discovery_filter() {
    let (mut view, _) = numerical();
    view.status = RunStatus::Completed as i32;
    view.completed_rounds = view.maximum_rounds;
    let mut child = golden().0.current_job.unwrap();
    child.status = DiscoveryJobStatus::Succeeded as i32;
    let research = view.research.as_mut().unwrap();
    research.stage = RunStage::Discovery as i32;
    research.current_child = Some(CurrentChild::Discovery(child));
    research.evaluation_job_id = None;
    research.disposition = RoundDisposition::Duplicate as i32;
    validate_view(&view).unwrap();
    view.research.as_mut().unwrap().disposition = RoundDisposition::PreviouslyRejected as i32;
    validate_view(&view).unwrap();
}

#[test]
fn exact_reservations() {
    let (view, wire) = golden();
    validate_view(&view).unwrap();
    assert_eq!(view.revision, 9_007_199_254_740_993);
    assert_eq!(view.reserved_input_tokens, 9_007_199_254_740_992);
    assert_eq!(
        view.reserved_cost
            .as_ref()
            .unwrap()
            .amount
            .as_ref()
            .unwrap()
            .value,
        "0.125"
    );
    assert_eq!(view.encode_to_vec(), wire);
}

#[test]
fn pending_advancement() {
    let (mut view, _) = golden();
    let child = view.current_job.as_mut().unwrap();
    child.status = DiscoveryJobStatus::Succeeded as i32;
    child.updated_at.as_mut().unwrap().seconds += 2;
    validate_view(&view).unwrap();
    view.plan_verified = false;
    validate_view(&view).unwrap();
}

#[test]
fn invalid_views() {
    let changes: &[fn(&mut RunView)] = &[
        |value| value.run_id.as_mut().unwrap().value = "run\n".into(),
        |value| {
            value
                .reserved_cost
                .as_mut()
                .unwrap()
                .amount
                .as_mut()
                .unwrap()
                .value = "1\n".into()
        },
        |value| value.status = 127,
        |value| value.status = 0,
        |value| value.revision = 0,
        |value| value.revision = u64::MAX,
        |value| value.budget = None,
        |value| value.current_job = None,
        |value| value.maximum_rounds = 65,
        |value| value.completed_rounds = 3,
        |value| value.completed_rounds = 2,
        |value| value.status = RunStatus::Completed as i32,
        |value| value.reserved_steps = 9,
        |value| value.reserved_input_tokens = 0,
        |value| value.reserved_output_tokens = 0,
        |value| value.budget.as_mut().unwrap().maximum_input_tokens = 0,
        |value| value.budget.as_mut().unwrap().maximum_output_tokens = 0,
        |value| {
            value
                .reserved_cost
                .as_mut()
                .unwrap()
                .amount
                .as_mut()
                .unwrap()
                .value = "0".into()
        },
        |value| value.reserved_input_tokens = 9_007_199_254_740_994,
        |value| value.reserved_output_tokens = 2049,
        |value| value.reserved_cost.as_mut().unwrap().currency_code = "EUR".into(),
        |value| {
            value
                .reserved_cost
                .as_mut()
                .unwrap()
                .amount
                .as_mut()
                .unwrap()
                .value = "0.6".into()
        },
        |value| {
            value
                .reserved_cost
                .as_mut()
                .unwrap()
                .amount
                .as_mut()
                .unwrap()
                .value = "0.1250".into()
        },
        |value| {
            value
                .reserved_cost
                .as_mut()
                .unwrap()
                .amount
                .as_mut()
                .unwrap()
                .value = "0.0000000001".into()
        },
        |value| {
            value
                .reserved_cost
                .as_mut()
                .unwrap()
                .amount
                .as_mut()
                .unwrap()
                .value = "-0.1".into()
        },
        |value| {
            value
                .budget
                .as_mut()
                .unwrap()
                .maximum_wall_time
                .as_mut()
                .unwrap()
                .nanos = 1
        },
        |value| value.deadline.as_mut().unwrap().seconds += 1,
        |value| value.updated_at.as_mut().unwrap().seconds -= 2,
        |value| value.submitted_at = None,
        |value| value.current_job.as_mut().unwrap().status = 127,
        |value| value.current_job.as_mut().unwrap().revision = 0,
        |value| {
            value
                .current_job
                .as_mut()
                .unwrap()
                .submitted_at
                .as_mut()
                .unwrap()
                .seconds -= 2
        },
        |value| value.run_id.as_mut().unwrap().value = "run bad".into(),
    ];
    for (index, change) in changes.iter().enumerate() {
        let (mut view, _) = golden();
        change(&mut view);
        assert!(
            validate_view(&view).is_err(),
            "accepted invalid case {index}"
        );
    }
}

#[test]
fn completed_projection() {
    let (mut view, _) = golden();
    view.status = RunStatus::Completed as i32;
    view.completed_rounds = 2;
    assert!(validate_view(&view).is_err());
    view.current_job.as_mut().unwrap().status = DiscoveryJobStatus::Succeeded as i32;
    validate_view(&view).unwrap();
}

#[test]
fn operator_surface() {
    let descriptor = FileDescriptorSet::decode(loop_protocol::FILE_DESCRIPTOR_SET).unwrap();
    let file = descriptor
        .file
        .iter()
        .find(|file| file.name() == "loop/runs/v1/service.proto")
        .unwrap();
    let methods = &file.service[0].method;
    assert_eq!(
        methods
            .iter()
            .map(|method| method.name())
            .collect::<Vec<_>>(),
        ["StartRun", "StepRun", "GetRun"]
    );
    let messages = descriptor
        .file
        .iter()
        .flat_map(|file| {
            file.message_type
                .iter()
                .map(move |message| (format!(".{}.{}", file.package(), message.name()), message))
        })
        .collect::<BTreeMap<_, _>>();
    let mut pending = methods
        .iter()
        .flat_map(|method| [method.input_type(), method.output_type()])
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if visited.insert(name)
            && let Some(message) = messages.get(name)
        {
            pending.extend(
                message
                    .field
                    .iter()
                    .filter_map(|field| field.type_name.as_deref()),
            );
        }
    }
    assert!(visited.contains(".loop.runs.v1.RunBudget"));
    assert!(visited.contains(".loop.discovery.v1.DiscoveryJobHandle"));
    for forbidden in [
        "RunSpecification",
        "DiscoveryJobInput",
        "DiscoveryCandidate",
        "JobSpecification",
        "HoldoutGrant",
        "ModelResolutionSnapshot",
    ] {
        assert!(
            !visited.iter().any(|name| name.ends_with(forbidden)),
            "exposed {forbidden}"
        );
    }
}
