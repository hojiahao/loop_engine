use crate::test_support as support;

mod fixture;
mod process;
mod runtime;

use std::sync::{Arc, atomic::AtomicI64};

use super::{LocalArtifacts, ObjectRef, files::ReadBudget, model};
use crate::store::{
    BacktestPolicy, BacktestRepository, FactorRepository, JobMutation, JobRepository, PgJobStore,
    StoreError,
};
use fixture::{Fixture, put};
use loop_protocol::wire::jobs::v1::{AcquireJobLeaseRequest, CompleteJobRequest};
use loop_protocol::wire::v1::{JobId, JobOutcome, JobState, job_outcome};
use support::{NOW, actor, context};

#[tokio::test]
async fn bytes_are_checked_before_resolution() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let success = fixture.success();
    assert!(
        policy
            .resolve_result(&fixture.job.specification, &success)
            .is_err()
    );
    let prepared = policy
        .prepare(
            &fixture.job.specification,
            Some(&fixture.context_id()),
            Some(&success),
        )
        .await
        .unwrap()
        .unwrap();
    let result = prepared
        .resolve_result(&fixture.job.specification, &success)
        .unwrap();
    assert_eq!(result.metrics[0].value.as_ref().unwrap().value, "1.25");
    assert_eq!(result.provenance, fixture.job_input().provenance);
    assert_eq!(
        result.result_manifest_sha256.unwrap().value,
        fixture.result_artifact.object.digest().unwrap()
    );
}

#[tokio::test]
async fn preparations_do_not_replace_each_other() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let success = fixture.success();
    let completed = policy
        .prepare(
            &fixture.job.specification,
            Some(&fixture.context_id()),
            Some(&success),
        )
        .await
        .unwrap()
        .unwrap();
    let queued = policy
        .prepare(&fixture.job.specification, None, None)
        .await
        .unwrap()
        .unwrap();
    assert!(
        queued
            .resolve_result(&fixture.job.specification, &success)
            .is_err()
    );
    assert!(
        queued
            .resolve_current(&actor(), &fixture.job.specification, &fixture.context_id())
            .is_err()
    );
    assert!(
        completed
            .resolve_result(&fixture.job.specification, &success)
            .is_ok()
    );
    assert!(
        completed
            .resolve_current(&actor(), &fixture.job.specification, &fixture.context_id())
            .is_ok()
    );
}

#[tokio::test]
async fn same_size_tampering_is_rejected() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let success = fixture.success();
    let prepared = policy
        .prepare(&fixture.job.specification, None, Some(&success))
        .await
        .unwrap()
        .unwrap();
    let path = fixture.path(&fixture.result.artifacts.nav.object);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0] ^= 1;
    std::fs::write(path, bytes).unwrap();
    assert!(
        prepared
            .resolve_result(&fixture.job.specification, &success)
            .is_err()
    );
    assert!(
        policy
            .prepare(&fixture.job.specification, None, Some(&success))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn replacing_a_path_invalidates_its_open_descriptor() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let success = fixture.success();
    let prepared = policy
        .prepare(&fixture.job.specification, None, Some(&success))
        .await
        .unwrap()
        .unwrap();
    let path = fixture.path(&fixture.result.artifacts.nav.object);
    let moved = fixture.directory.path().join("replaced-object");
    std::fs::rename(&path, moved).unwrap();
    std::fs::write(path, b"different bytes").unwrap();
    assert!(
        prepared
            .resolve_result(&fixture.job.specification, &success)
            .is_err()
    );
}

#[tokio::test]
async fn missing_dependency_invalidates_current_context() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let success = fixture.success();
    let prepared = policy
        .prepare(
            &fixture.job.specification,
            Some(&fixture.context_id()),
            Some(&success),
        )
        .await
        .unwrap()
        .unwrap();
    std::fs::remove_file(fixture.path(&fixture.context.calendar)).unwrap();
    assert!(
        prepared
            .resolve_current(&actor(), &fixture.job.specification, &fixture.context_id())
            .is_err()
    );
}

#[tokio::test]
async fn unknown_and_spoofed_readers_are_denied() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let prepared = policy
        .prepare(
            &fixture.job.specification,
            Some(&fixture.context_id()),
            None,
        )
        .await
        .unwrap()
        .unwrap();
    let mut spoofed = actor();
    spoofed.actor_id.as_mut().unwrap().value = "other.actor".to_owned();
    assert!(matches!(
        prepared.resolve_current(&spoofed, &fixture.job.specification, &fixture.context_id()),
        Err(StoreError::AdmissionDenied)
    ));
    assert!(
        prepared
            .resolve_current(&actor(), &fixture.job.specification, "latest")
            .is_err()
    );
}

#[tokio::test]
async fn catalog_aliases_cannot_rebind_contexts() {
    let mut fixture = Fixture::new();
    fixture.catalog.contexts[0].context_id = "latest".to_owned();
    let reference = fixture.json(&fixture.catalog);
    assert!(
        super::TrustedManifests::load(
            LocalArtifacts::open(&fixture.root).unwrap(),
            reference,
            vec![fixture.registry.clone()],
            vec![actor()]
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn freeze_binds_the_seed() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let mut job = fixture.job.specification.clone();
    let Some(loop_protocol::wire::v1::job_specification::Input::Backtest(input)) = &mut job.input
    else {
        unreachable!()
    };
    input.deterministic_seed.as_mut().unwrap().value[0] ^= 1;
    assert!(policy.prepare(&job, None, None).await.is_err());
}

#[tokio::test]
async fn catalog_cannot_change_the_frozen_engine() {
    for engine in [
        model::Engine::PrimaryCrossSectional,
        model::Engine::ZiplineValidation,
    ] {
        let mut fixture = Fixture::new();
        let clock = Arc::new(support::FixtureClock(AtomicI64::new(NOW)));
        let store = fixture.open(clock.clone(), fixture.policy().await).await;
        store.submit(fixture.job.clone()).await.unwrap();
        store.close().await;
        let mut specification: model::Backtest = serde_json::from_slice(
            &std::fs::read(fixture.path(&fixture.catalog.backtests[0].specification)).unwrap(),
        )
        .unwrap();
        specification.engine = engine;
        if engine == model::Engine::PrimaryCrossSectional {
            specification.engine_version = "changed-after-submission".to_owned();
        }
        fixture.catalog.backtests[0].specification = fixture.json(&specification);
        let store = fixture.open(clock, fixture.policy().await).await;
        assert!(
            store
                .mutate(
                    &actor(),
                    JobMutation::Acquire(AcquireJobLeaseRequest {
                        context: Some(context("manifest.changed-engine")),
                        job_id: fixture.job.specification.job_id.clone(),
                        expected_revision: 1,
                        requested_duration: Some(prost_types::Duration {
                            seconds: 30,
                            nanos: 0
                        }),
                    })
                )
                .await
                .is_err()
        );
        assert_eq!(
            store.get("job.1").await.unwrap().unwrap().state,
            JobState::Queued as i32
        );
        assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 1);
        store.close().await;
    }
}

#[tokio::test]
async fn recent_dates_cannot_claim_in_sample_role() {
    let mut fixture = Fixture::new();
    let mut data: model::Dataset =
        serde_json::from_slice(&std::fs::read(fixture.path(&fixture.context.data)).unwrap())
            .unwrap();
    data.sample.start = "2025-01-01".to_owned();
    data.sample.end = "2026-08-31".to_owned();
    assert!(matches!(
        data.sample.validate(),
        Err(StoreError::AdmissionDenied)
    ));
    let mut changed = fixture.context.clone();
    changed.data = fixture.json(&data);
    fixture.replace_context(changed);
    let policy = fixture.policy().await;
    assert!(matches!(
        policy.prepare(&fixture.job.specification, None, None).await,
        Err(StoreError::AdmissionDenied)
    ));
}

#[tokio::test]
async fn review_cannot_relax_the_frozen_coverage_policy() {
    let mut fixture = Fixture::new();
    fixture.review.minimum_coverage_bps = 100;
    fixture.republish_result();
    let policy = fixture.policy().await;
    assert!(
        policy
            .prepare(&fixture.job.specification, None, Some(&fixture.success()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn family_cannot_perturb_a_non_window_argument() {
    let mut fixture = Fixture::new();
    let mut family: model::Family = serde_json::from_slice(
        &std::fs::read(fixture.path(fixture.context.family.as_ref().unwrap())).unwrap(),
    )
    .unwrap();
    family.window_path = vec![0];
    let mut context = fixture.context.clone();
    context.family = Some(fixture.json(&family));
    fixture.replace_context(context);
    let policy = fixture.policy().await;
    assert!(
        policy
            .prepare(
                &fixture.job.specification,
                Some(&fixture.context_id()),
                None
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn verified_installed_worker_advances_durable_state() {
    use crate::research_worker::{PythonPerturber, ResearchBuild};
    use crate::store::PerturbationRepository;
    use serde::Deserialize;

    let mut fixture = Fixture::new();
    let python = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/python");
    let capture = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::process::Command::new(&python)
            .args([
                "-I",
                "-m",
                "loop_research.cli",
                "build-manifests",
                "--store",
            ])
            .arg(&fixture.root)
            .env_clear()
            .env("OPENBLAS_NUM_THREADS", "1")
            .env("OMP_NUM_THREADS", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        capture.status.success(),
        "{}",
        String::from_utf8_lossy(&capture.stderr)
    );
    #[derive(Deserialize)]
    struct Build {
        source: ObjectRef,
        environment: ObjectRef,
    }
    let captured: Build = serde_json::from_slice(&capture.stdout).unwrap();
    let build = ResearchBuild {
        source_sha256: captured.source.digest().unwrap(),
        environment_sha256: captured.environment.digest().unwrap(),
    };
    let mut context = fixture.context.clone();
    context.source = captured.source;
    context.environment = captured.environment;
    fixture.replace_context(context);
    let policy = fixture.policy().await;
    let store = fixture
        .open(
            Arc::new(support::FixtureClock(AtomicI64::new(NOW))),
            policy.clone(),
        )
        .await;
    seed(&fixture, &store).await;
    let mut command = support::perturbation::command(1, 0, "manifest.perturb");
    command.context_id = fixture.context_id();
    assert!(
        store
            .advance_perturbation(
                &actor(),
                command.clone(),
                &PythonPerturber::new(&python).unwrap()
            )
            .await
            .is_err()
    );
    let worker = PythonPerturber::verified(&python, build).unwrap();
    let result = store
        .advance_perturbation(&actor(), command.clone(), &worker)
        .await
        .unwrap();
    assert_eq!(result.revision, 1);
    assert_eq!(result.step.state.as_ref().unwrap().history.len(), 1);
    assert_ne!(result.step.candidate.as_ref().unwrap().window, 5);
    assert!(
        store
            .advance_perturbation(&actor(), command, &worker)
            .await
            .unwrap()
            .replayed
    );
    store.close().await;
}

#[tokio::test]
async fn canonical_family_has_real_distinct_factor_ids() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let prepared = policy
        .prepare(
            &fixture.job.specification,
            Some(&fixture.context_id()),
            None,
        )
        .await
        .unwrap()
        .unwrap();
    let family = prepared
        .resolve_perturbation_space(&actor(), &fixture.job.specification, &fixture.context_id())
        .unwrap();
    assert_eq!(
        family
            .candidates
            .iter()
            .map(|candidate| candidate.window)
            .collect::<Vec<_>>(),
        vec![5, 10, 15]
    );
    assert_ne!(
        family.candidates[0].factor_spec_id,
        family.candidates[1].factor_spec_id
    );
    assert_eq!(
        family.candidates[0].factor_spec_id,
        fixture.job_input().factor_spec_id
    );
    assert!(
        prepared
            .validate_worker(family.provenance.as_ref().unwrap(), None)
            .is_err()
    );
}

#[tokio::test]
async fn result_manifest_cannot_name_a_different_job() {
    let mut fixture = Fixture::new();
    fixture.result.job_id = "job.other".to_owned();
    fixture.republish_result();
    let policy = fixture.policy().await;
    assert!(
        policy
            .prepare(&fixture.job.specification, None, Some(&fixture.success()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn result_engine_must_match_the_freeze() {
    let mut fixture = Fixture::new();
    fixture.result.engine_version = "different-build".to_owned();
    fixture.republish_result();
    let policy = fixture.policy().await;
    assert!(
        policy
            .prepare(&fixture.job.specification, None, Some(&fixture.success()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn output_schema_cannot_be_substituted() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let mut success = fixture.success();
    success.outputs[0].schema.as_mut().unwrap().name = "other.schema".to_owned();
    assert!(
        policy
            .prepare(&fixture.job.specification, None, Some(&success))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn strict_json_rejects_duplicate_or_unknown_fields() {
    let directory = tempfile::Builder::new()
        .prefix("loop-manifests-")
        .tempdir()
        .unwrap();
    let store = LocalArtifacts::open(directory.path()).unwrap();
    for bytes in [
        br#"{"sha256":"x","sha256":"y","byte_size":0}"#.as_slice(),
        br#"{"sha256":"x","byte_size":0,"extra":true}"#.as_slice(),
    ] {
        let reference = put(directory.path(), bytes);
        let file = store
            .load(&reference, true, &mut ReadBudget::new())
            .await
            .unwrap();
        assert!(file.json::<ObjectRef>().is_err());
    }
}

#[tokio::test]
async fn symlinks_and_special_files_are_rejected() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::Builder::new()
        .prefix("loop-manifests-")
        .tempdir()
        .unwrap();
    let store = LocalArtifacts::open(directory.path()).unwrap();
    let reference = ObjectRef {
        sha256: format!("sha256:{}", "a".repeat(64)),
        byte_size: 0,
    };
    let path = directory.path().join("a".repeat(64));
    symlink("/dev/null", &path).unwrap();
    assert!(
        store
            .load(&reference, false, &mut ReadBudget::new())
            .await
            .is_err()
    );
    std::fs::remove_file(&path).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &path,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR,
        0,
    )
    .unwrap();
    assert!(
        store
            .load(&reference, false, &mut ReadBudget::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn traversal_and_oversized_metadata_are_rejected() {
    let directory = tempfile::Builder::new()
        .prefix("loop-manifests-")
        .tempdir()
        .unwrap();
    let store = LocalArtifacts::open(directory.path()).unwrap();
    for reference in [
        ObjectRef {
            sha256: "../../other-project".to_owned(),
            byte_size: 1,
        },
        ObjectRef {
            sha256: format!("sha256:{}", "a".repeat(64)),
            byte_size: 2_000_000,
        },
    ] {
        assert!(
            store
                .load(&reference, true, &mut ReadBudget::new())
                .await
                .is_err()
        );
    }
}

async fn lease(fixture: &Fixture, store: &PgJobStore) -> loop_protocol::wire::v1::JobRecord {
    store.submit(fixture.job.clone()).await.unwrap();
    store
        .mutate(
            &actor(),
            JobMutation::Acquire(AcquireJobLeaseRequest {
                context: Some(context("manifest.acquire")),
                job_id: Some(JobId {
                    value: "job.1".to_owned(),
                }),
                expected_revision: 1,
                requested_duration: Some(prost_types::Duration {
                    seconds: 30,
                    nanos: 0,
                }),
            }),
        )
        .await
        .unwrap()
        .job
}

async fn seed(fixture: &Fixture, store: &PgJobStore) {
    let leased = lease(fixture, store).await;
    store
        .mutate(
            &actor(),
            JobMutation::Complete(CompleteJobRequest {
                context: Some(context("manifest.complete")),
                job_id: leased.specification.unwrap().job_id,
                expected_revision: leased.revision,
                lease_id: leased.active_lease.unwrap().lease_id,
                outcome: Some(JobOutcome {
                    outcome: Some(job_outcome::Outcome::Success(fixture.success())),
                }),
            }),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn missing_files_do_not_hide_infrastructure_failure() {
    use loop_protocol::wire::v1::{ErrorCategory, InfrastructureFailure, ServiceError};
    let fixture = Fixture::new();
    let store = fixture
        .open(
            Arc::new(support::FixtureClock(AtomicI64::new(NOW))),
            fixture.policy().await,
        )
        .await;
    let leased = lease(&fixture, &store).await;
    std::fs::remove_file(fixture.path(&fixture.context.data)).unwrap();
    let command = JobMutation::Complete(CompleteJobRequest {
        context: Some(context("manifest.infrastructure-failed")),
        job_id: leased.specification.unwrap().job_id,
        expected_revision: leased.revision,
        lease_id: leased.active_lease.unwrap().lease_id,
        outcome: Some(JobOutcome {
            outcome: Some(job_outcome::Outcome::InfrastructureFailure(
                InfrastructureFailure {
                    error: Some(ServiceError {
                        category: ErrorCategory::Dependency as i32,
                        code: "artifact_unavailable".to_owned(),
                        message: "Research artifact is unavailable".to_owned(),
                        retryable: false,
                        details: vec![],
                    }),
                    attempt: leased.attempt,
                    failed_at: Some(support::timestamp(NOW)),
                },
            )),
        }),
    });
    let result = store.mutate(&actor(), command.clone()).await.unwrap();
    assert_eq!(result.job.state, JobState::InfrastructureFailed as i32);
    assert!(store.mutate(&actor(), command).await.unwrap().replayed);
    assert!(
        store
            .current_backtest(&actor(), "job.1", &fixture.context_id())
            .await
            .is_err()
    );
    let events = store.audit_events(0, 100).await.unwrap();
    loop_core::audit::verify_audit_chain(&events).unwrap();
    assert_eq!(events.len(), 3);
    store.close().await;
}

#[tokio::test]
async fn acquisition_rechecks_files_under_the_lock() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct TamperingClock {
        path: std::path::PathBuf,
        armed: AtomicBool,
    }
    impl crate::store::Clock for TamperingClock {
        fn now_millis(&self) -> crate::store::StoreResult<i64> {
            if self.armed.swap(false, Ordering::SeqCst) {
                std::fs::remove_file(&self.path).unwrap();
            }
            Ok(NOW)
        }
    }
    let fixture = Fixture::new();
    let clock = Arc::new(TamperingClock {
        path: fixture.path(&fixture.context.calendar),
        armed: AtomicBool::new(false),
    });
    let store = fixture.open(clock.clone(), fixture.policy().await).await;
    store.submit(fixture.job.clone()).await.unwrap();
    clock.armed.store(true, Ordering::SeqCst);
    assert!(
        store
            .mutate(
                &actor(),
                JobMutation::Acquire(AcquireJobLeaseRequest {
                    context: Some(context("manifest.acquire-tampered")),
                    job_id: fixture.job.specification.job_id.clone(),
                    expected_revision: 1,
                    requested_duration: Some(prost_types::Duration {
                        seconds: 30,
                        nanos: 0
                    }),
                })
            )
            .await
            .is_err()
    );
    let record = store.get("job.1").await.unwrap().unwrap();
    assert_eq!(record.state, JobState::Queued as i32);
    assert_eq!(record.revision, 1);
    assert!(record.active_lease.is_none());
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 1);
    store.close().await;
}

#[tokio::test]
async fn registered_manifest_survives_restart_and_audited_export() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let clock = Arc::new(support::FixtureClock(AtomicI64::new(NOW)));
    let store = fixture.open(clock.clone(), policy).await;
    seed(&fixture, &store).await;
    store.close().await;
    let policy = fixture.policy().await;
    let store = fixture.open(clock, policy.clone()).await;
    let mut command = support::backtest::export("manifest.export");
    command.context_id = fixture.context_id();
    let mut output = Vec::new();
    policy
        .write_current(&store, &actor(), command.clone(), &mut output)
        .await
        .unwrap();
    let export: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(export["data_quality"], "synthetic");
    assert_eq!(export["result_manifest"]["metrics"][0]["value"], "1.25");
    assert!(
        policy
            .write_current(&store, &actor(), command, &mut Vec::new())
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(
        store.get("job.1").await.unwrap().unwrap().state,
        JobState::Succeeded as i32
    );
    let events = store.audit_events(0, 100).await.unwrap();
    loop_core::audit::verify_audit_chain(&events).unwrap();
    assert_eq!(events.len(), 4);
    store.close().await;
}

#[tokio::test]
async fn actual_review_enters_the_shared_admission_handler() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let store = fixture
        .open(Arc::new(support::FixtureClock(AtomicI64::new(NOW))), policy)
        .await;
    seed(&fixture, &store).await;
    let mut command = support::library::command(1, 0, "manifest.admit");
    command.context_id = fixture.context_id();
    let decision = store.decide_factor(&actor(), command).await.unwrap();
    assert_eq!(decision.states[0].status, "admitted");
    assert_eq!(
        decision.states[0].factor_spec_id,
        fixture.job_input().factor_spec_id.unwrap().value
    );
    store.close().await;
}

#[tokio::test]
async fn corrupted_files_cannot_release_an_export() {
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let store = fixture
        .open(
            Arc::new(support::FixtureClock(AtomicI64::new(NOW))),
            policy.clone(),
        )
        .await;
    seed(&fixture, &store).await;
    std::fs::write(
        fixture.path(&fixture.result.artifacts.simple_returns.object),
        b"corrupt",
    )
    .unwrap();
    let mut command = support::backtest::export("manifest.export");
    command.context_id = fixture.context_id();
    let mut output = Vec::new();
    assert!(
        policy
            .write_current(&store, &actor(), command, &mut output)
            .await
            .is_err()
    );
    assert!(output.is_empty());
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 3);
    store.close().await;
}

#[tokio::test]
async fn actual_manifest_changes_invalidate_all_six_components() {
    use loop_protocol::provenance::{ProvenanceComponent, ProvenanceError};
    let mut fixture = Fixture::new();
    let extra_registry = Arc::new(fixture::registry(true));
    let mut contexts = Vec::new();
    for component in ProvenanceComponent::ALL {
        let mut changed = fixture.context.clone();
        changed.family = None;
        match component {
            ProvenanceComponent::SourceCode | ProvenanceComponent::Environment => {
                let role = if component == ProvenanceComponent::SourceCode {
                    "source"
                } else {
                    "environment"
                };
                let reference = fixture.json(&model::FileSet {
                    schema: format!("loop.{role}-files/v1"),
                    files: vec![model::NamedFile {
                        name: "changed-build.txt".to_owned(),
                        object: put(&fixture.root, b"changed implementation"),
                    }],
                });
                if role == "source" {
                    changed.source = reference;
                } else {
                    changed.environment = reference;
                }
            }
            ProvenanceComponent::OperatorRegistry => {
                changed.registry = put(&fixture.root, &extra_registry.canonical_bytes());
            }
            ProvenanceComponent::Configuration => {
                let mut configuration: model::Configuration = serde_json::from_slice(
                    &std::fs::read(fixture.path(&changed.configuration)).unwrap(),
                )
                .unwrap();
                let document = fixture.json(&model::PolicyDocument {
                    schema: "loop.research-policy/v1".to_owned(),
                    policy_id: "policy.extra".to_owned(),
                    revision: "1".to_owned(),
                    settings: std::collections::BTreeMap::new(),
                });
                configuration.policies.push(model::Policy {
                    policy_id: "policy.extra".to_owned(),
                    revision: "1".to_owned(),
                    document,
                });
                configuration
                    .policies
                    .sort_by(|left, right| left.policy_id.cmp(&right.policy_id));
                changed.configuration = fixture.json(&configuration);
            }
            ProvenanceComponent::DataManifest => {
                let mut data: model::Dataset =
                    serde_json::from_slice(&std::fs::read(fixture.path(&changed.data)).unwrap())
                        .unwrap();
                data.snapshots[0].source = "synthetic-v2".to_owned();
                changed.data = fixture.json(&data);
            }
            ProvenanceComponent::TradingCalendar => {
                let mut calendar: model::Calendar = serde_json::from_slice(
                    &std::fs::read(fixture.path(&changed.calendar)).unwrap(),
                )
                .unwrap();
                calendar.sessions.push("2016-01-11".to_owned());
                changed.calendar = fixture.json(&calendar);
            }
        }
        let reference = fixture.json(&changed);
        contexts.push((reference.sha256.clone(), component));
        fixture.catalog.contexts.push(model::ContextEntry {
            context_id: reference.sha256.clone(),
            manifest: reference,
        });
    }
    fixture
        .catalog
        .contexts
        .sort_by(|left, right| left.context_id.cmp(&right.context_id));
    let policy = Arc::new(
        super::TrustedManifests::load(
            LocalArtifacts::open(&fixture.root).unwrap(),
            fixture.json(&fixture.catalog),
            vec![fixture.registry.clone(), extra_registry],
            vec![actor()],
        )
        .await
        .unwrap(),
    );
    let store = fixture
        .open(Arc::new(support::FixtureClock(AtomicI64::new(NOW))), policy)
        .await;
    seed(&fixture, &store).await;
    for (context, component) in contexts {
        assert!(
            matches!(store.current_backtest(&actor(), "job.1", &context).await,
            Err(StoreError::Provenance(ProvenanceError::Stale(fields))) if fields == [component])
        );
    }
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 3);
    store.close().await;
}

#[tokio::test]
async fn writer_failure_preserves_the_committed_acceptance() {
    use std::pin::Pin;
    use std::task::{Context, Poll};
    struct Broken;
    impl tokio::io::AsyncWrite for Broken {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Err(std::io::Error::other("fixture sink unavailable")))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let store = fixture
        .open(
            Arc::new(support::FixtureClock(AtomicI64::new(NOW))),
            policy.clone(),
        )
        .await;
    seed(&fixture, &store).await;
    let mut command = support::backtest::export("manifest.sink-failure");
    command.context_id = fixture.context_id();
    assert!(matches!(
        policy
            .write_current(&store, &actor(), command.clone(), &mut Broken)
            .await,
        Err(StoreError::Unavailable("export writer"))
    ));
    assert!(
        policy
            .write_current(&store, &actor(), command, &mut Vec::new())
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 4);
    store.close().await;
}

#[tokio::test]
async fn cancelled_delivery_replays_without_another_acceptance() {
    use tokio::io::AsyncReadExt;
    let fixture = Fixture::new();
    let policy = fixture.policy().await;
    let store = fixture
        .open(
            Arc::new(support::FixtureClock(AtomicI64::new(NOW))),
            policy.clone(),
        )
        .await;
    seed(&fixture, &store).await;
    let mut command = support::backtest::export("manifest.cancelled-delivery");
    command.context_id = fixture.context_id();
    let (mut writer, mut reader) = tokio::io::duplex(1);
    let principal = actor();
    let mut delivery =
        Box::pin(policy.write_current(&store, &principal, command.clone(), &mut writer));
    tokio::select! {
        result = &mut delivery => panic!("delivery must block: {result:?}"),
        byte = reader.read_u8() => { byte.unwrap(); }
        () = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
            panic!("export did not reach delivery");
        }
    }
    // Dropping the unfinished future cancels delivery, not its committed receipt.
    drop(delivery);
    assert!(
        policy
            .write_current(&store, &principal, command, &mut Vec::new())
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(store.audit_events(0, 100).await.unwrap().len(), 4);
    store.close().await;
}
