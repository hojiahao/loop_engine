//! Installed validators exercised over the real mTLS/PostgreSQL command path.

use super::*;
use crate::manifests::reconciliation::{ComparisonPolicy, ValidationDocument};
use crate::manifests::tests::fixture::artifact;
use crate::runtime::{ReconciliationConfig, ValidationPin};
use crate::store::SubmitJob;

mod processes;

pub(super) fn price(row: usize, column: usize) -> String {
    // Deliberately varied cross-sectional returns; no market-data claim.
    let bps = ((row * 13 + column * 7 + row * column * 11) % 71) as i64 - 30;
    format!("{:.3}", (10 + column) as f64 * (1.0 + bps as f64 / 1000.0))
}

pub(super) fn extend_panel(f: &mut Fixture) {
    let days = [
        4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 19, 20, 21, 22, 25, 26, 27,
    ];
    let sessions: Vec<_> = days.iter().map(|day| format!("2010-01-{day:02}")).collect();
    let securities: Vec<_> = (1..=6).map(|value| format!("US.{value:03}")).collect();
    let mut decisions = Vec::new();
    let mut csv = "session,security_id,eligible,known_at_ms,market.close\n".to_owned();
    for (row, day) in days.iter().enumerate() {
        let known = chrono::NaiveDate::from_ymd_opt(2010, 1, *day)
            .unwrap()
            .and_hms_opt(21, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        decisions.push(known + 300_000);
        for (column, security) in securities.iter().enumerate() {
            csv.push_str(&format!(
                "2010-01-{day:02},{security},1,{known},{}\n",
                price(row, column)
            ));
        }
    }
    let values = artifact(
        &f.root,
        "loop.factor_panel_values",
        "text/csv",
        csv.as_bytes(),
        &[
            "session",
            "security_id",
            "eligible",
            "known_at_ms",
            "market.close",
        ],
    );
    #[derive(serde::Serialize)]
    struct Panel<'a> {
        schema: &'a str,
        quality: &'a str,
        sessions: &'a [String],
        securities: &'a [String],
        fields: [&'a str; 1],
        decision_times_ms: &'a [i64],
        evaluation_start: &'a str,
        values: &'a ObjectRef,
    }
    let panel = serde_json::to_vec(&Panel {
        schema: "loop.factor-panel/v1",
        quality: "synthetic",
        sessions: &sessions,
        securities: &securities,
        fields: ["market.close"],
        decision_times_ms: &decisions,
        // The first session warms up ma(2); unavailable warmup values must not
        // enter the independent statistical acceptance sample.
        evaluation_start: "2010-01-05",
        values: &values.object,
    })
    .unwrap();
    let data = model::Dataset {
        schema: "loop.development-dataset/v1".to_owned(),
        sample: model::Sample {
            role: model::SampleRole::InSample,
            start: "2010-01-05".to_owned(),
            end: "2010-01-27".to_owned(),
        },
        quality: model::Quality::Synthetic,
        snapshots: vec![model::Snapshot {
            snapshot_id: "snapshot.validation".to_owned(),
            source: "synthetic".to_owned(),
            dataset: "independent-acceptance".to_owned(),
            entitlement: "synthetic".to_owned(),
            known_through_ms: *decisions.last().unwrap(),
            artifacts: vec![
                artifact(
                    &f.root,
                    "loop.factor_panel",
                    "application/json",
                    &panel,
                    &[],
                ),
                values,
            ],
        }],
    };
    f.context.data = f.json(&data);
    f.context.calendar = f.json(&model::Calendar {
        schema: "loop.trading-calendar/v1".to_owned(),
        name: "XNYS".to_owned(),
        timezone: "America/New_York".to_owned(),
        sessions,
    });
}

struct ValidationCase {
    portfolio: PortfolioCase,
    job: JobSpecification,
    output: PathBuf,
}

impl ValidationCase {
    async fn new(extended: bool) -> Self {
        let mut portfolio = PortfolioCase::with_history(extended).await;
        let f = &portfolio.base.fixture;
        let policy = ComparisonPolicy {
            schema: "loop.reconciliation-policy/v1".to_owned(),
            policy_id: "policy.reconciliation".to_owned(),
            revision: "1".to_owned(),
            profile: "alphalens-zipline-development.1".to_owned(),
            statistics_absolute: "0.000000000001".to_owned(),
            statistics_relative: "0.0000000001".to_owned(),
            price_absolute: "0.000000005".to_owned(),
            dollar_absolute: "0.00001".to_owned(),
            return_absolute: "0.000000000001".to_owned(),
            accounting_relative: "0".to_owned(),
        };
        let reference = f.json(&policy);
        let mut job = portfolio.job.clone();
        job.job_id.as_mut().unwrap().value = "job.validation".to_owned();
        job.idempotency_key.as_mut().unwrap().value = "submit.validation".to_owned();
        job.kind = JobKind::IndependentReconciliation as i32;
        let Some(job_specification::Input::Backtest(primary)) = &portfolio.job.input else {
            unreachable!()
        };
        job.input = Some(job_specification::Input::Reconciliation(
            ReconciliationJobInput {
                primary_backtest_id: None,
                independent_backtest_id: None,
                reconciliation_policy: Some(PolicyReference {
                    policy_id: Some(PolicyId {
                        value: policy.policy_id,
                    }),
                    revision: policy.revision,
                    sha256: Some(Sha256Digest {
                        value: reference.digest().unwrap().to_vec(),
                    }),
                }),
                budget: primary.budget.clone(),
                validation: Some(IndependentValidationSource {
                    primary_job_id: portfolio.job.job_id.clone(),
                    context_manifest_sha256: Some(Sha256Digest {
                        value: hex_bytes(&portfolio.context_id),
                    }),
                }),
            },
        ));
        let output = f.directory.path().join("validation-output");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let uv = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join("uv"))
            .find(|path| path.is_file())
            .expect("installed uv");
        let python = std::process::Command::new(&uv)
            .args(["python", "find", "--offline", "3.12.13"])
            .env("UV_PYTHON_INSTALL_DIR", root.join(".tools/python"))
            .output()
            .unwrap();
        assert!(
            python.status.success(),
            "Zipline interpreter must be installed by bootstrap"
        );
        portfolio.validation = Some(ReconciliationConfig {
            uv,
            zipline_python: PathBuf::from(String::from_utf8(python.stdout).unwrap().trim()),
            alphalens_project: root.join("python/alphalens_validation"),
            zipline_project: root.join("python/zipline_validation"),
            cache: root.join(".tools/uv-cache"),
            output_store: output.clone(),
            jobs: vec![ValidationPin {
                job_id: "job.validation".to_owned(),
                primary_job_id: "job.portfolio".to_owned(),
                policy: reference,
            }],
        });
        portfolio.jobs.push(job.clone());
        portfolio.restart().await;
        portfolio
            .base
            .store
            .submit(SubmitJob {
                specification: job.clone(),
                request_id: "request.validation".to_owned(),
            })
            .await
            .unwrap();
        Self {
            portfolio,
            job,
            output,
        }
    }

    async fn primary(&self) {
        self.portfolio
            .base
            .client()
            .await
            .execute_backtest(self.portfolio.request_seconds(300).await)
            .await
            .unwrap();
    }

    async fn client(&self) -> job_service_client::JobServiceClient<tonic::transport::Channel> {
        self.portfolio
            .base
            .tls
            .timed_client(
                self.portfolio.base.address,
                Some("client"),
                Duration::from_secs(240),
            )
            .await
            .unwrap()
    }

    async fn request(&self) -> ExecuteReconciliationRequest {
        let record = self
            .client()
            .await
            .acquire_job_lease(AcquireJobLeaseRequest {
                context: Some(context("validation.acquire")),
                job_id: self.job.job_id.clone(),
                expected_revision: 1,
                requested_duration: Some(prost_types::Duration {
                    seconds: 300,
                    nanos: 0,
                }),
            })
            .await
            .unwrap()
            .into_inner()
            .job
            .unwrap();
        ExecuteReconciliationRequest {
            context: Some(context("validation.execute")),
            job_id: self.job.job_id.clone(),
            lease_id: record.active_lease.unwrap().lease_id,
            expected_revision: record.revision,
        }
    }

    fn document(&self, job: &JobRecord) -> ValidationDocument {
        let Some(job_outcome::Outcome::Success(success)) =
            job.outcome.as_ref().unwrap().outcome.as_ref()
        else {
            panic!("completed validation required")
        };
        let reference = &success.outputs[0].artifact_id.as_ref().unwrap().value;
        serde_json::from_slice(&std::fs::read(self.output.join(&reference[7..])).unwrap()).unwrap()
    }

    fn summary(&self, reference: &ObjectRef) -> String {
        let receipt: serde_json::Value = serde_json::from_slice(
            &std::fs::read(self.output.join(&reference.sha256[7..])).unwrap(),
        )
        .unwrap();
        let summary = receipt["summary"]["sha256"].as_str().unwrap();
        std::fs::read_to_string(self.output.join(&summary[7..])).unwrap()
    }

    fn decision(&self, revision: u64) -> DecideFactorRequest {
        let mut request = self.portfolio.decide_request(revision);
        request.deadline.as_mut().unwrap().seconds = request
            .context
            .as_ref()
            .unwrap()
            .requested_at
            .as_ref()
            .unwrap()
            .seconds
            + 180;
        request
    }
}

fn hex_bytes(value: &str) -> Vec<u8> {
    value.as_bytes()[7..]
        .chunks_exact(2)
        .map(|part| u8::from_str_radix(std::str::from_utf8(part).unwrap(), 16).unwrap())
        .collect()
}

#[tokio::test]
async fn accepted_replay() {
    let mut case = ValidationCase::new(true).await;
    case.primary().await;
    let request = case.request().await;
    let first = case
        .client()
        .await
        .execute_reconciliation(request.clone())
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    assert_eq!(first.state, JobState::Succeeded as i32);
    let document = case.document(&first);
    assert!(
        document.disposition == crate::manifests::reconciliation::Disposition::Accepted,
        "Alphalens: {}\nZipline: {}",
        case.summary(&document.alphalens),
        case.summary(&document.zipline),
    );
    assert!(!document.production_eligible);
    let count = case
        .portfolio
        .base
        .store
        .audit_events(0, 100)
        .await
        .unwrap()
        .len();
    case.portfolio.restart().await;
    let replay = case
        .client()
        .await
        .execute_reconciliation(request)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(
        case.portfolio
            .base
            .store
            .audit_events(0, 100)
            .await
            .unwrap()
            .len(),
        count
    );
    let current = case
        .portfolio
        .operator()
        .await
        .read_reconciliation(ReadReconciliationRequest {
            job_id: case.job.job_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(current.job, Some(first));
    for revision in [0, 1] {
        let error = case
            .portfolio
            .operator()
            .await
            .decide_factor(case.decision(revision))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::FailedPrecondition);
        assert!(
            error.message().contains("production admission requires"),
            "{error}"
        );
    }
    let mut forced = case.decision(0);
    forced.context = Some(case.portfolio.operator_context("validation.force"));
    forced.override_reason = "A semantic override cannot grant production data".to_owned();
    forced.override_approval_id = "approval.nonexistent".to_owned();
    let error = case
        .portfolio
        .operator()
        .await
        .decide_factor(forced)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert!(error.message().contains("production admission requires"));
}

#[tokio::test]
async fn imported_validation_denied() {
    let case = ValidationCase::new(false).await;
    let request = case.request().await;
    let command = CompleteJobRequest {
        context: request.context,
        job_id: request.job_id,
        lease_id: request.lease_id,
        expected_revision: request.expected_revision,
        outcome: Some(JobOutcome {
            outcome: Some(job_outcome::Outcome::Success(
                case.portfolio.base.fixture.success(),
            )),
        }),
    };
    let error = case
        .client()
        .await
        .complete_job(command.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    // Also exercise the storage boundary without relying on missing RPC
    // capability metadata to deny the imported result earlier in transport.
    assert!(matches!(
        case.portfolio
            .base
            .store
            .mutate(&actor(), crate::store::JobMutation::Complete(command))
            .await,
        Err(crate::store::StoreError::AdmissionDenied)
    ));
}

#[tokio::test]
async fn unregistered_primary_denied() {
    let case = ValidationCase::new(false).await;
    let error = case
        .client()
        .await
        .execute_reconciliation(case.request().await)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn unavailable_registered() {
    let case = ValidationCase::new(false).await;
    case.primary().await;
    let result = case
        .client()
        .await
        .execute_reconciliation(case.request().await)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    assert_eq!(result.state, JobState::Succeeded as i32);
    assert!(
        case.document(&result).disposition
            == crate::manifests::reconciliation::Disposition::Unavailable
    );
    let error = case
        .portfolio
        .operator()
        .await
        .decide_factor(case.decision(0))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert_eq!(
        error.message(),
        "independent portfolio reconciliation unavailable"
    );
}

#[tokio::test]
async fn changed_trial_denied() {
    let mut case = ValidationCase::new(false).await;
    case.primary().await;
    case.client()
        .await
        .execute_reconciliation(case.request().await)
        .await
        .unwrap();
    case.portfolio.add_trial().await;
    let error = case
        .portfolio
        .operator()
        .await
        .read_reconciliation(ReadReconciliationRequest {
            job_id: case.job.job_id.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Aborted);
    assert!(error.message().contains("trial"));
}

#[tokio::test]
async fn corrupted_report_denied() {
    let case = ValidationCase::new(false).await;
    case.primary().await;
    let completed = case
        .client()
        .await
        .execute_reconciliation(case.request().await)
        .await
        .unwrap()
        .into_inner()
        .job
        .unwrap();
    let document = case.document(&completed);
    let path = case.output.join(&document.alphalens.sha256[7..]);
    std::fs::write(&path, b"corrupted").unwrap();
    let error = case
        .portfolio
        .operator()
        .await
        .read_reconciliation(ReadReconciliationRequest {
            job_id: case.job.job_id.clone(),
        })
        .await
        .unwrap_err();
    assert!(
        [Code::DataLoss, Code::Unavailable].contains(&error.code()),
        "{error}"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"corrupted");
    let mut connection = support::connection(&case.portfolio.base.fixture.directory).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM factor_states")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn invalid_validation_deadline() {
    let case = ValidationCase::new(false).await;
    let mut request = case.decision(0);
    request.deadline.as_mut().unwrap().seconds += 1;
    let error = case
        .portfolio
        .operator()
        .await
        .decide_factor(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(std::fs::read_dir(&case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn expired_validation_denied() {
    let case = ValidationCase::new(false).await;
    let request = case.request().await;
    case.portfolio.base.clock.0.store(301_000, Ordering::SeqCst);
    let error = case
        .client()
        .await
        .execute_reconciliation(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert_eq!(std::fs::read_dir(case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn cancelled_validation_denied() {
    let case = ValidationCase::new(false).await;
    let request = case.request().await;
    case.portfolio
        .operator()
        .await
        .cancel_job(CancelJobRequest {
            context: Some(case.portfolio.operator_context("cancel.validation")),
            job_id: case.job.job_id.clone(),
            expected_revision: request.expected_revision,
            reason: "Fence independent worker".to_owned(),
        })
        .await
        .unwrap();
    let error = case
        .client()
        .await
        .execute_reconciliation(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Aborted);
    assert_eq!(std::fs::read_dir(case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn actor_spoof_denied() {
    let case = ValidationCase::new(false).await;
    let mut request = case.request().await;
    request.context.as_mut().unwrap().actor = Some(case.portfolio.operator_actor());
    let error = case
        .client()
        .await
        .execute_reconciliation(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(std::fs::read_dir(case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn spoofed_decision_denied() {
    let case = ValidationCase::new(false).await;
    let mut request = case.decision(0);
    request.context.as_mut().unwrap().actor = Some(actor());
    let error = case
        .portfolio
        .operator()
        .await
        .decide_factor(request)
        .await
        .unwrap_err();
    // Attribution must be checked before looking for a completed comparison or
    // starting numerical replay, even when the certificate can read the source.
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(std::fs::read_dir(case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn operator_execution_denied() {
    let case = ValidationCase::new(false).await;
    let request = case.request().await;
    let error = case
        .portfolio
        .operator()
        .await
        .execute_reconciliation(request)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(std::fs::read_dir(case.output).unwrap().count(), 0);
}

#[tokio::test]
async fn protected_validation_denied() {
    let running = super::super::super::Running::start_with(Role::HoldoutWorker, true).await;
    let error = running
        .client()
        .await
        .execute_reconciliation(ExecuteReconciliationRequest {
            context: Some(context("validation.protected")),
            job_id: running.id(),
            lease_id: Some(LeaseId {
                value: "lease.unavailable".to_owned(),
            }),
            expected_revision: 1,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 0);
}

#[tokio::test]
async fn unconfigured_validation_denied() {
    let running = super::super::super::Running::start(Role::Research).await;
    let job = running.acquire().await;
    let error = running
        .client()
        .await
        .execute_reconciliation(ExecuteReconciliationRequest {
            context: Some(support::context("validation.unconfigured")),
            job_id: running.id(),
            lease_id: job.active_lease.unwrap().lease_id,
            expected_revision: job.revision,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(std::fs::read_dir(&running.views).unwrap().count(), 0);
}
