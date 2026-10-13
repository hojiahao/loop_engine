use super::*;
use crate::manifests::numerical::tests::Case;
use crate::test_support as fixture;

async fn record(case: &Case, bytes: &[u8]) -> (wire::JobRecord, ObjectRef) {
    let mut files = Vec::new();
    let object = case
        .resolver
        .publish(bytes, false, &mut files)
        .await
        .unwrap();
    let reference = artifact(
        &case.resolver,
        object.clone(),
        "loop.perturbation_work",
        "application/x-protobuf",
        fixture::NOW - 1_000,
        false,
        &mut files,
    )
    .await
    .unwrap();
    let (_, wire::job_specification::Input::Discovery(discovery)) =
        fixture::research::inputs().remove(0)
    else {
        unreachable!()
    };
    let mut job = fixture::command(1).specification;
    job.kind = wire::JobKind::Perturbation as i32;
    job.input = Some(wire::job_specification::Input::Perturbation(
        wire::PerturbationJobInput {
            dataset: discovery.dataset,
            feedback_policy: discovery.research_policy,
            parent_binding_sha256: Some(fixture::digest(31)),
            family_sha256: Some(fixture::digest(32)),
            work: Some(reference),
            source_code_sha256: Some(fixture::digest(33)),
            environment_sha256: Some(fixture::digest(34)),
            budget: Some(wire::JobBudget {
                maximum_steps: 1,
                maximum_cost: Some(wire::Money {
                    amount: Some(wire::ExactDecimal {
                        value: "0".to_owned(),
                    }),
                    currency_code: "USD".to_owned(),
                }),
                maximum_wall_time: Some(prost_types::Duration {
                    seconds: 20,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            algorithm: "window.ema-gradient-pcg64.v1".to_owned(),
        },
    ));
    let selection = job.protocol_selection.as_mut().unwrap();
    selection
        .enabled_features
        .push("jobs.perturbation.v1".to_owned());
    selection.enabled_features.sort_unstable();
    selection.enabled_features.dedup();
    selection.selection_sha256 = Some(wire::Sha256Digest {
        value: loop_protocol::job::protocol_selection_sha256(selection)
            .unwrap()
            .to_vec(),
    });
    loop_protocol::job::validate_job_specification(&job).unwrap();
    (
        wire::JobRecord {
            specification: Some(job),
            ..Default::default()
        },
        object,
    )
}

fn work() -> wire::PerturbationWork {
    // This fixture exercises input decoding only; it grants no worker or
    // completion authority and deliberately omits a prepared family/state.
    wire::PerturbationWork {
        current_window: 2,
        ..Default::default()
    }
}

#[tokio::test]
async fn work_replay() {
    let case = Case::new();
    let expected = work();
    let (record, object) = record(&case, &expected.encode_to_vec()).await;
    assert_eq!(case.resolver.read_work(&record).await.unwrap(), expected);
    std::fs::remove_file(case.fixture.path(&object)).unwrap();
    assert!(case.resolver.read_work(&record).await.is_err());
    assert!(!case.fixture.path(&object).exists());
}

#[tokio::test]
async fn work_tampering() {
    let case = Case::new();
    let (record, object) = record(&case, &work().encode_to_vec()).await;
    case.resolver.read_work(&record).await.unwrap();
    let mut changed = work();
    changed.current_window = 3;
    // Equal size must still fail the hash after invalidating the cached guard.
    std::fs::write(case.fixture.path(&object), changed.encode_to_vec()).unwrap();
    assert!(case.resolver.read_work(&record).await.is_err());
}

#[tokio::test]
async fn work_canonical() {
    let case = Case::new();
    let mut bytes = work().encode_to_vec();
    // An unknown protobuf field decodes but must not be silently discarded.
    bytes.extend_from_slice(&[0xf8, 0x07, 0x01]);
    let (record, _) = record(&case, &bytes).await;
    assert!(matches!(
        case.resolver.read_work(&record).await,
        Err(StoreError::Corrupt("noncanonical perturbation protobuf"))
    ));
}

#[tokio::test]
async fn work_schema() {
    let case = Case::new();
    let (record, _) = record(&case, &work().encode_to_vec()).await;
    let Some(wire::job_specification::Input::Perturbation(input)) =
        &record.specification.as_ref().unwrap().input
    else {
        unreachable!()
    };
    let schema = input.work.as_ref().unwrap().schema.as_ref().unwrap();
    let object = ObjectRef {
        sha256: format!(
            "sha256:{}",
            schema
                .schema_sha256
                .as_ref()
                .unwrap()
                .value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
        byte_size: 1,
    };
    std::fs::remove_file(case.fixture.path(&object)).unwrap();
    assert!(case.resolver.read_work(&record).await.is_err());
    assert!(!case.fixture.path(&object).exists());
}
