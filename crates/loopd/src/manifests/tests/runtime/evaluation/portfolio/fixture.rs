use super::*;
use crate::manifests::tests::fixture::{artifact, put};
use crate::store::SubmitJob;
use std::collections::BTreeMap;

pub(super) struct PortfolioCase {
    pub base: Case,
    pub job: JobSpecification,
    pub context_id: String,
    pub(super) pin: PortfolioPin,
    pub(super) output: PathBuf,
    pub(super) jobs: Vec<JobSpecification>,
    pub(super) validation: Option<crate::runtime::ReconciliationConfig>,
    pub(super) statistics: Option<crate::runtime::StatisticsConfig>,
    pub(super) extra_pins: Vec<PortfolioPin>,
}

impl PortfolioCase {
    pub(super) async fn new() -> Self {
        Self::with_history(false).await
    }

    pub(super) async fn with_history(extended: bool) -> Self {
        let mut f = fixture_with_minimum(false, 6666).await;
        if extended {
            super::reconciliation::extend_panel(&mut f);
        }
        configure(&mut f, extended);
        let base = Case::open(f).await;
        let request = base.request().await;
        base.client().await.evaluate_factor(request).await.unwrap();
        let source = &base.fixture.job.specification;
        let trial = base
            .store
            .factor_trials(&actor(), &source.run_id.as_ref().unwrap().value, "", 10)
            .await
            .unwrap()
            .remove(0)
            .evaluation
            .unwrap();
        let evaluated = trial.result.as_ref().unwrap();
        let resolver = EvaluationResolver::open(
            &base.fixture.root,
            vec![EvaluationPin {
                job_id: source.job_id.as_ref().unwrap().value.clone(),
                context: base.fixture.json(&base.fixture.context),
            }],
        )
        .unwrap();
        let work = resolver
            .prepare(source, evaluated.lease_id.as_ref().unwrap())
            .await
            .unwrap()
            .work;
        let f = &base.fixture;
        let work_ref = put(&f.root, &work.encode_to_vec());
        let copy = |reference: &ArtifactRef| ObjectRef {
            sha256: put(
                &f.root,
                &std::fs::read(
                    base.output
                        .join(&reference.artifact_id.as_ref().unwrap().value[7..]),
                )
                .unwrap(),
            )
            .sha256,
            byte_size: reference.byte_size,
        };
        let evaluation = copy(evaluated.manifest.as_ref().unwrap());
        let values = copy(evaluated.values.as_ref().unwrap());
        let mut configuration: model::Configuration =
            serde_json::from_slice(&std::fs::read(f.path(&f.context.configuration)).unwrap())
                .unwrap();
        let documents: BTreeMap<String, model::PolicyDocument> = configuration
            .policies
            .iter()
            .map(|policy| {
                let document: model::PolicyDocument =
                    serde_json::from_slice(&std::fs::read(f.path(&policy.document)).unwrap())
                        .unwrap();
                (
                    format!(
                        "{}_policy",
                        policy.policy_id.strip_prefix("policy.").unwrap()
                    ),
                    document,
                )
            })
            .collect();
        let data: model::Dataset =
            serde_json::from_slice(&std::fs::read(f.path(&f.context.data)).unwrap()).unwrap();
        let tape = &data
            .snapshots
            .iter()
            .find(|snapshot| snapshot.snapshot_id == "snapshot.execution")
            .unwrap()
            .artifacts[0]
            .object;
        let recipe = f.json(&serde_json::json!({"schema": "loop.portfolio-request/v1", "evaluation_work": work_ref, "evaluation_result": evaluation, "factor_values": values, "execution_tape": tape, "policies": documents}));
        configuration.backtest_engine_version = "authorized-portfolio.1".to_owned();
        configuration.portfolio_request = Some(recipe.clone());
        let mut context_manifest = f.context.clone();
        context_manifest.configuration = f.json(&configuration);
        let context_ref = f.json(&context_manifest);
        let factor = work.factor.as_ref().unwrap();
        let factor_ref = f.json(&model::Factor {
            schema: "loop.factor-manifest/v1".to_owned(),
            factor_spec_id: factor.factor_spec_id.as_ref().unwrap().value.clone(),
            specification: put(
                &f.root,
                &loop_protocol::job::factor_identity_bytes(factor).unwrap(),
            ),
            expression: put(&f.root, &factor.expression.as_ref().unwrap().canonical_json),
        });
        let specification = f.json(&model::Backtest {
            schema: "loop.backtest-spec/v1".to_owned(),
            backtest_id: "backtest.portfolio".to_owned(),
            context: context_ref.clone(),
            factor: factor_ref,
            engine: model::Engine::PrimaryCrossSectional,
            engine_version: "authorized-portfolio.1".to_owned(),
            sample: data.sample,
            return_definition: "simple_nav_return".to_owned(),
            deterministic_seed: format!("sha256:{}", "01".repeat(32)),
        });
        let mut job = source.clone();
        job.job_id.as_mut().unwrap().value = "job.portfolio".to_owned();
        job.idempotency_key.as_mut().unwrap().value = "submit.portfolio".to_owned();
        job.kind = JobKind::Backtest as i32;
        let Some(job_specification::Input::FactorEvaluation(input)) = &source.input else {
            unreachable!()
        };
        job.input = Some(job_specification::Input::Backtest(BacktestJobInput {
            factor_spec_id: factor.factor_spec_id.clone(),
            dataset: input.dataset.clone(),
            return_definition: ReturnDefinition::SimpleNavReturn as i32,
            provenance: Some(
                context_manifest
                    .provenance(*f.registry.identity().as_bytes())
                    .unwrap(),
            ),
            deterministic_seed: input.deterministic_seed.clone(),
            budget: input.budget.clone(),
        }));
        let pin = PortfolioPin {
            job_id: "job.portfolio".to_owned(),
            specification,
            request: recipe,
            evaluation_job_id: source.job_id.as_ref().unwrap().value.clone(),
        };
        let output = f.directory.path().join("portfolio-output");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
        let jobs = vec![source.clone(), job.clone()];
        let mut case = Self {
            base,
            job,
            context_id: context_ref.sha256,
            pin,
            output,
            jobs,
            validation: None,
            statistics: None,
            extra_pins: vec![],
        };
        case.restart().await;
        case.base
            .store
            .submit(SubmitJob {
                specification: case.job.clone(),
                request_id: "request.portfolio".to_owned(),
            })
            .await
            .unwrap();
        case
    }

    pub(super) fn operator_actor(&self) -> Actor {
        Actor {
            actor_id: Some(ActorId {
                value: "actor.operator".to_owned(),
            }),
            kind: ActorKind::Human as i32,
            display_name: "Portfolio operator".to_owned(),
            authenticated_subject: "subject.operator".to_owned(),
        }
    }

    pub(super) fn operator_context(&self, key: &str) -> CommandContext {
        let mut command = context(key);
        command.actor = Some(self.operator_actor());
        command
    }

    pub(super) fn export_request(&self, key: &str) -> ExportBacktestRequest {
        let context = self.operator_context(key);
        let mut deadline = context.requested_at.unwrap();
        deadline.seconds += 30;
        ExportBacktestRequest {
            context: Some(context),
            job_id: self.job.job_id.clone(),
            context_id: self.context_id.clone(),
            deadline: Some(deadline),
        }
    }

    pub(super) fn decide_request(&self, revision: u64) -> DecideFactorRequest {
        let context = self.operator_context(&format!("portfolio.decide.{revision}"));
        let mut deadline = context.requested_at.unwrap();
        deadline.seconds += 30;
        DecideFactorRequest {
            context: Some(context),
            source_job_id: self.job.job_id.clone(),
            context_id: self.context_id.clone(),
            expected_revision: revision,
            reason: "Primary result awaits independent reconciliation".to_owned(),
            deadline: Some(deadline),
            ..Default::default()
        }
    }

    pub(super) async fn operator(
        &self,
    ) -> job_service_client::JobServiceClient<tonic::transport::Channel> {
        self.base
            .tls
            .timed_client(self.base.address, Some("unknown"), Duration::from_secs(240))
            .await
            .unwrap()
    }

    pub(super) async fn restart(&mut self) {
        let c = &mut self.base;
        c.task.abort();
        let digest = std::process::Command::new("openssl")
            .args(["x509", "-in"])
            .arg(c.tls.path("unknown.pem"))
            .args(["-outform", "DER"])
            .output()
            .unwrap();
        assert!(digest.status.success());
        let now = SystemClock.now_millis().unwrap();
        let worker = actor();
        let operator = Actor {
            actor_id: Some(ActorId {
                value: "actor.operator".to_owned(),
            }),
            kind: ActorKind::Human as i32,
            display_name: "Portfolio operator".to_owned(),
            authenticated_subject: "subject.operator".to_owned(),
        };
        let mut runs: Vec<_> = self
            .jobs
            .iter()
            .map(|job| job.run_id.as_ref().unwrap().value.clone())
            .collect();
        runs.sort();
        runs.dedup();
        let identities = [
            (worker, Role::Research, c.tls.client_digest()),
            (
                operator,
                Role::Operator,
                format!("sha256:{:x}", Sha256::digest(digest.stdout)),
            ),
        ]
        .into_iter()
        .map(|(actor, role, certificate)| Identity {
            actor_id: actor.actor_id.unwrap().value,
            subject: actor.authenticated_subject,
            display_name: actor.display_name,
            role,
            certificate_sha256: vec![certificate],
            not_before_ms: now - 1000,
            expires_at_ms: now + 3_600_000,
            run_ids: runs.clone(),
        })
        .collect();
        c.authority = Arc::new(
            RuntimeAuthority::new(
                identities,
                self.jobs
                    .iter()
                    .map(|job| JobPin {
                        job_id: job.job_id.as_ref().unwrap().value.clone(),
                        specification_sha256: format!(
                            "sha256:{:x}",
                            Sha256::digest(job.encode_to_vec())
                        ),
                    })
                    .collect(),
                c.clock.clone(),
            )
            .unwrap(),
        );
        c.broker = Arc::new(
            ArtifactBroker::open(
                &c.fixture.root,
                &c.fixture.directory.path().join("protected"),
                &c.views,
                self.jobs
                    .iter()
                    .map(|job| DataPin {
                        job_id: job.job_id.as_ref().unwrap().value.clone(),
                        manifest: c.fixture.context.data.clone(),
                        protected: false,
                    })
                    .collect(),
            )
            .unwrap(),
        );
        let executor = Arc::new(
            PortfolioExecutor::open(
                &python(),
                &c.fixture.root,
                &self.output,
                std::iter::once(self.pin.clone())
                    .chain(self.extra_pins.iter().cloned())
                    .collect(),
                c.broker.clone(),
            )
            .unwrap(),
        );
        let mut options = support::base_options(&c.fixture.directory.path().join("state"));
        options.clock = c.clock.clone();
        options.admission = c.authority.clone();
        options.backtest_policy = executor.clone();
        c.store = PgJobStore::open(options).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        c.address = listener.local_addr().unwrap();
        let mut service =
            RuntimeService::new(c.store.clone(), c.authority.clone(), c.broker.clone())
                .with_factor_executor(c.executor.clone())
                .with_portfolio_executor(executor.clone());
        if let Some(config) = &self.validation {
            service = service.with_reconciler(Arc::new(
                crate::runtime::ReconciliationExecutor::open(config.clone(), executor.clone())
                    .unwrap(),
            ));
        }
        if let Some(config) = &self.statistics {
            service = service.with_statistician(Arc::new(
                crate::runtime::StatisticsExecutor::open(config.clone(), executor).unwrap(),
            ));
        }
        c.task = tokio::spawn(crate::runtime::serve(
            service,
            listener,
            c.tls.server(),
            std::future::pending(),
        ));
    }

    pub(super) async fn request(&self) -> ExecuteBacktestRequest {
        self.request_seconds(180).await
    }

    pub(super) async fn request_seconds(&self, seconds: i64) -> ExecuteBacktestRequest {
        let result = self
            .base
            .client()
            .await
            .acquire_job_lease(AcquireJobLeaseRequest {
                context: Some(context("portfolio.acquire")),
                job_id: self.job.job_id.clone(),
                expected_revision: 1,
                requested_duration: Some(prost_types::Duration { seconds, nanos: 0 }),
            })
            .await
            .unwrap()
            .into_inner()
            .job
            .unwrap();
        ExecuteBacktestRequest {
            context: Some(context("portfolio.execute")),
            job_id: self.job.job_id.clone(),
            lease_id: result.active_lease.unwrap().lease_id,
            expected_revision: result.revision,
        }
    }

    pub(super) async fn record(&self) -> JobRecord {
        self.base.store.get("job.portfolio").await.unwrap().unwrap()
    }

    pub(super) async fn add_trial(&mut self) {
        let mut job = self.jobs[0].clone();
        job.job_id.as_mut().unwrap().value = "job.other".to_owned();
        job.run_id.as_mut().unwrap().value = "run.other".to_owned();
        job.idempotency_key.as_mut().unwrap().value = "submit.other".to_owned();
        if let Some(job_specification::Input::FactorEvaluation(input)) = &mut job.input {
            input.deterministic_seed.as_mut().unwrap().value[0] ^= 1;
        }
        self.jobs.push(job.clone());
        self.restart().await;
        self.base
            .store
            .submit(SubmitJob {
                specification: job,
                request_id: "request.other".to_owned(),
            })
            .await
            .unwrap();
    }
}

fn configure(f: &mut Fixture, extended: bool) {
    let mut configuration: model::Configuration =
        serde_json::from_slice(&std::fs::read(f.path(&f.context.configuration)).unwrap()).unwrap();
    for entry in &mut configuration.policies {
        let mut document: model::PolicyDocument =
            serde_json::from_slice(&std::fs::read(f.path(&entry.document)).unwrap()).unwrap();
        let settings: &[(&str, &str)] = match entry.policy_id.as_str() {
            "policy.portfolio" => &[
                ("algorithm", "long-only-top-n.1"),
                ("holdings", "1"),
                ("initial_cash_usd", "1000"),
                ("lot_size", "1"),
            ],
            "policy.execution" => &[("algorithm", "next-session-open.1")],
            "policy.cost" => &[
                ("algorithm", "commission-spread.1"),
                ("commission_per_share_usd", "0"),
                ("minimum_commission_usd", "0"),
                ("half_spread_bps", "0"),
            ],
            "policy.evaluation" => &[
                ("minimum_coverage_bps", "6666"),
                ("statistics_profile", "daily-statistics.1"),
                ("groups", "2"),
                ("minimum_cross_section", "3"),
                ("minimum_sessions", "8"),
                ("hac_lags", "1"),
                ("pbo_blocks", "4"),
            ],
            _ => &[],
        };
        document.settings = settings
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        entry.document = f.json(&document);
    }
    f.context.configuration = f.json(&configuration);
    let mut data: model::Dataset =
        serde_json::from_slice(&std::fs::read(f.path(&f.context.data)).unwrap()).unwrap();
    let panel = data.snapshots[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.schema.name == "loop.factor_panel")
        .unwrap();
    let panel: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.path(&panel.object)).unwrap()).unwrap();
    let mut csv =
        "session,security_id,open_at_ms,open_usd,close_known_at_ms,close_usd\n".to_owned();
    for (index, day) in panel["sessions"].as_array().unwrap().iter().enumerate() {
        let day = day.as_str().unwrap();
        if day < panel["evaluation_start"].as_str().unwrap() {
            continue;
        }
        let date = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        let opening = date
            .and_hms_opt(14, 30, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let closing = date
            .and_hms_opt(21, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        for (column, security) in panel["securities"].as_array().unwrap().iter().enumerate() {
            csv.push_str(&format!(
                "{day},{},{opening},{},{closing},{}\n",
                security.as_str().unwrap(),
                10 + column,
                if extended {
                    super::reconciliation::price(index, column)
                } else {
                    (10 + column + index).to_string()
                }
            ));
        }
    }
    let observations = artifact(
        &f.root,
        "loop.execution_observations",
        "text/csv",
        csv.as_bytes(),
        &[
            "session",
            "security_id",
            "open_at_ms",
            "open_usd",
            "close_known_at_ms",
            "close_usd",
        ],
    );
    let tape = serde_json::to_vec(&serde_json::json!({"schema":"loop.execution-tape/v1", "quality":"synthetic", "currency":"USD", "price_basis":"raw", "corporate_actions":"none_in_sample_declared", "observations":observations.object})).unwrap();
    data.snapshots.push(model::Snapshot {
        snapshot_id: "snapshot.execution".to_owned(),
        source: "synthetic".to_owned(),
        dataset: "execution".to_owned(),
        entitlement: "synthetic".to_owned(),
        known_through_ms: data.snapshots[0].known_through_ms,
        artifacts: vec![
            artifact(
                &f.root,
                "loop.execution_tape",
                "application/json",
                &tape,
                &[],
            ),
            observations,
        ],
    });
    data.snapshots
        .sort_by(|left, right| left.snapshot_id.cmp(&right.snapshot_id));
    f.context.data = f.json(&data);
    let Some(job_specification::Input::FactorEvaluation(input)) = &mut f.job.specification.input
    else {
        unreachable!()
    };
    let factor = input.factor.as_mut().unwrap();
    let frozen = factor.frozen_policy.as_mut().unwrap();
    for policy in [
        &mut frozen.universe_policy,
        &mut frozen.data_policy,
        &mut frozen.calendar_policy,
        &mut frozen.preprocess_policy,
        &mut frozen.neutralization_policy,
        &mut frozen.portfolio_policy,
        &mut frozen.execution_policy,
        &mut frozen.cost_policy,
        &mut frozen.evaluation_policy,
    ] {
        let policy = policy.as_mut().unwrap();
        let entry = configuration
            .policies
            .iter()
            .find(|entry| entry.policy_id == policy.policy_id.as_ref().unwrap().value)
            .unwrap();
        policy.sha256 = Some(model::digest(entry.document.digest().unwrap()));
    }
    factor.factor_spec_id.as_mut().unwrap().value = format!(
        "sha256:{}",
        loop_protocol::job::factor_identity_hash(factor)
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    input.dataset = Some(data.reference(&f.context.data).unwrap());
    input.provenance = Some(
        f.context
            .provenance(*f.registry.identity().as_bytes())
            .unwrap(),
    );
}
