#![cfg(feature = "runs-service")]

use loop_protocol::job::validate_job_specification;
use loop_protocol::runs::validate_view;
use loop_protocol::wire::research::v1::ResearchJobStatus;
use loop_protocol::wire::runs::v1::{
    RoundDisposition, RunStatus, RunView, research_run_view::CurrentChild,
};
use loop_protocol::wire::v1::{JobSpecification, PerturbationWork, job_specification};
use prost::Message;

fn wire(field: &str) -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/protocol/v1/adaptive_workflow_v3.json"
    ))
    .unwrap();
    fixture[field]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|bytes| u8::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap())
        .collect()
}

fn view() -> RunView {
    RunView::decode(wire("view_wire_hex").as_slice()).unwrap()
}
fn job() -> JobSpecification {
    JobSpecification::decode(wire("job_wire_hex").as_slice()).unwrap()
}

#[test]
fn adaptive_golden() {
    let view = view();
    let job = job();
    validate_view(&view).unwrap();
    validate_job_specification(&job).unwrap();
    // Protobuf field order is not canonical across producers. In particular,
    // prost emits the input oneof (tag 16) before envelope tags 10–15, whereas
    // the Python fixture orders fields by tag. Preserve every decoded value.
    assert_eq!(
        RunView::decode(view.encode_to_vec().as_slice()).unwrap(),
        view
    );
    assert_eq!(
        JobSpecification::decode(job.encode_to_vec().as_slice()).unwrap(),
        job
    );
}

#[test]
fn observed_golden() {
    let work = PerturbationWork::decode(wire("observed_work_wire_hex").as_slice()).unwrap();
    assert_eq!(work.observed_factor_ids.len(), 2);
    assert!(work.failed_factor_ids.is_empty());
    assert!(work.state.as_ref().unwrap().proposed_factor_ids.is_empty());
    for (candidate, observed) in work.candidates.iter().zip(&work.observed_factor_ids) {
        assert_eq!(candidate.factor_spec_id.as_ref(), Some(observed));
    }
    assert_eq!(
        PerturbationWork::decode(work.encode_to_vec().as_slice()).unwrap(),
        work
    );
}

#[test]
fn adaptive_guards() {
    let changes: &[fn(&mut RunView)] = &[
        |view| view.adaptive = None,
        |view| view.adaptive.as_mut().unwrap().feedback_policy = None,
        |view| view.adaptive.as_mut().unwrap().source_count = 17,
        |view| view.adaptive.as_mut().unwrap().source_count = 2,
        |view| view.adaptive.as_mut().unwrap().feedback_sha256 = None,
        |view| view.adaptive.as_mut().unwrap().model_context_sha256 = None,
        |view| view.adaptive.as_mut().unwrap().family_sha256 = None,
        |view| view.adaptive.as_mut().unwrap().state_sha256 = None,
        |view| view.adaptive.as_mut().unwrap().random_draws = 97,
        |view| {
            let research = view.research.as_mut().unwrap();
            research.discovery_job_id = research.perturbation_job_id.clone();
        },
        |view| view.research.as_mut().unwrap().perturbation_job_id = None,
        |view| view.research.as_mut().unwrap().round = 1,
        |view| view.status = RunStatus::SearchExhausted as i32,
    ];
    for (index, mutate) in changes.iter().enumerate() {
        let mut value = view();
        mutate(&mut value);
        assert!(
            validate_view(&value).is_err(),
            "accepted adaptive case {index}"
        );
    }
}

#[test]
fn perturbation_guards() {
    use loop_protocol::wire::v1::PerturbationJobInput;
    let changes: &[fn(&mut PerturbationJobInput)] = &[
        |input| input.dataset = None,
        |input| input.feedback_policy = None,
        |input| input.parent_binding_sha256 = None,
        |input| input.family_sha256 = None,
        |input| input.source_code_sha256 = None,
        |input| input.environment_sha256 = None,
        |input| input.work = None,
        |input| input.work.as_mut().unwrap().byte_size = 1_048_577,
        |input| input.algorithm = "unknown".into(),
        |input| input.budget = None,
        |input| input.budget.as_mut().unwrap().maximum_steps = 2,
        |input| input.budget.as_mut().unwrap().maximum_input_tokens = 1,
        |input| {
            input
                .budget
                .as_mut()
                .unwrap()
                .maximum_wall_time
                .as_mut()
                .unwrap()
                .seconds = 21
        },
    ];
    for (index, mutate) in changes.iter().enumerate() {
        let mut value = job();
        let Some(job_specification::Input::Perturbation(input)) = value.input.as_mut() else {
            panic!("fixture input")
        };
        mutate(input);
        assert!(
            validate_job_specification(&value).is_err(),
            "accepted perturbation case {index}"
        );
    }
}

#[test]
fn search_exhaustion() {
    let mut view = view();
    view.status = RunStatus::SearchExhausted as i32;
    view.completed_rounds = 2;
    let research = view.research.as_mut().unwrap();
    research.disposition = RoundDisposition::Exhausted as i32;
    let Some(CurrentChild::Research(child)) = research.current_child.as_mut() else {
        panic!("fixture handle")
    };
    child.status = ResearchJobStatus::Succeeded as i32;
    validate_view(&view).unwrap();
    assert!(view.completed_rounds < view.maximum_rounds);
    let Some(CurrentChild::Research(child)) =
        view.research.as_mut().unwrap().current_child.as_mut()
    else {
        panic!("fixture handle")
    };
    child.status = ResearchJobStatus::FactorRejected as i32;
    assert!(validate_view(&view).is_err());
}

#[test]
fn legacy_reinterpretation() {
    let job = JobSpecification::decode([0x18, 0x05, 0x82, 0x01, 0x00].as_slice()).unwrap();
    assert!(matches!(
        job.input,
        Some(job_specification::Input::Perturbation(_))
    ));
    assert!(validate_job_specification(&job).is_err());
}
