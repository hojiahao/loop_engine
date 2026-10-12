#![cfg(feature = "runs-service")]

use std::collections::{BTreeMap, BTreeSet};

use loop_protocol::runs::validate_view;
use loop_protocol::wire::discovery::v1::DiscoveryJobStatus;
use loop_protocol::wire::runs::v1::{RunStatus, RunView};
use prost::Message;
use prost_types::FileDescriptorSet;

fn golden() -> (RunView, Vec<u8>) {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/protocol/v1/run_view_v1.json"
    ))
    .unwrap();
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
