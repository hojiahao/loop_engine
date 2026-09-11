//! Synthetic producer metadata, real canonical factors, files and ledger commands.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loop_core::factor::*;
use loop_protocol::wire::v1::{
    Actor, BacktestJobInput, JobKind, JobRecord, JobSpecification, JobSuccess, ReturnDefinition,
    job_specification,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::super::{LocalArtifacts, ObjectRef, TrustedManifests, model};
use super::support::{self, NOW, actor};
use crate::store::{AdmissionPolicy, Clock, PgJobStore, StoreError, StoreResult, SubmitJob};

pub(super) struct Fixture {
    pub directory: TempDir,
    pub root: PathBuf,
    pub registry: Arc<OperatorPolicyRegistry>,
    pub catalog: model::Catalog,
    pub context: model::Context,
    pub result: model::ResultManifest,
    pub result_artifact: model::Artifact,
    pub job: SubmitJob,
    pub review: model::Review,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("loop-manifests-")
            .tempdir()
            .unwrap();
        let root = directory.path().join("objects");
        std::fs::create_dir(&root).unwrap();
        let registry = Arc::new(registry(false));
        let source = put_json(
            &root,
            &model::FileSet {
                schema: "loop.source-files/v1".to_owned(),
                files: vec![model::NamedFile {
                    name: "synthetic-producer.txt".to_owned(),
                    object: put(
                        &root,
                        b"Synthetic result producer, not market-data computation.",
                    ),
                }],
            },
        );
        let environment = put_json(
            &root,
            &model::FileSet {
                schema: "loop.environment-files/v1".to_owned(),
                files: vec![model::NamedFile {
                    name: "synthetic-environment.txt".to_owned(),
                    object: put(&root, b"Not an installed numerical worker."),
                }],
            },
        );
        let policies = [
            "universe",
            "data",
            "calendar",
            "preprocess",
            "neutralization",
            "portfolio",
            "execution",
            "cost",
            "evaluation",
        ]
        .map(|role| {
            let policy_id = format!("policy.{role}");
            let document = put_json(
                &root,
                &model::PolicyDocument {
                    schema: "loop.research-policy/v1".to_owned(),
                    policy_id: policy_id.clone(),
                    revision: "1".to_owned(),
                    settings: if role == "evaluation" {
                        BTreeMap::from([("minimum_coverage_bps".to_owned(), "9500".to_owned())])
                    } else {
                        BTreeMap::new()
                    },
                },
            );
            model::Policy {
                policy_id,
                revision: "1".to_owned(),
                document,
            }
        });
        let mut sorted_policies = policies.to_vec();
        sorted_policies.sort_by(|left, right| left.policy_id.cmp(&right.policy_id));
        let configuration = put_json(
            &root,
            &model::Configuration {
                schema: "loop.research-configuration/v1".to_owned(),
                backtest_engine: model::Engine::PrimaryCrossSectional,
                backtest_engine_version: "synthetic-producer.1".to_owned(),
                policies: sorted_policies,
            },
        );
        let sample = model::Sample {
            role: model::SampleRole::InSample,
            start: "2016-01-04".to_owned(),
            end: "2016-01-08".to_owned(),
        };
        let data = model::Dataset {
            schema: "loop.development-dataset/v1".to_owned(),
            sample: sample.clone(),
            quality: model::Quality::Synthetic,
            snapshots: vec![model::Snapshot {
                snapshot_id: "snapshot.manifest".to_owned(),
                source: "synthetic".to_owned(),
                dataset: "unit-test".to_owned(),
                entitlement: "synthetic".to_owned(),
                known_through_ms: 1_452_286_800_000,
                artifacts: vec![artifact(
                    &root,
                    "loop.synthetic_prices",
                    "text/csv",
                    b"session,security,close\n2016-01-04,security.1,100\n",
                    &[],
                )],
            }],
        };
        let data_object = put_json(&root, &data);
        let calendar = put_json(
            &root,
            &model::Calendar {
                schema: "loop.trading-calendar/v1".to_owned(),
                name: "XNYS".to_owned(),
                timezone: "America/New_York".to_owned(),
                sessions: [
                    "2016-01-04",
                    "2016-01-05",
                    "2016-01-06",
                    "2016-01-07",
                    "2016-01-08",
                ]
                .map(str::to_owned)
                .to_vec(),
            },
        );
        let factors = [5, 10, 15].map(|window| factor(&root, &registry, &policies, window));
        let family = put_json(
            &root,
            &model::Family {
                schema: "loop.window-family/v1".to_owned(),
                algorithm: "window.ema-gradient-pcg64.v1".to_owned(),
                window_path: vec![1],
                random_seed: format!("sha256:{}", "00".repeat(32)),
                backtest_seed: format!("sha256:{}", "09".repeat(32)),
                candidates: [5, 10, 15]
                    .into_iter()
                    .zip(factors.iter())
                    .map(|(window, (_, factor))| model::Candidate {
                        window,
                        factor: factor.clone(),
                    })
                    .collect(),
            },
        );
        let context = model::Context {
            schema: "loop.research-context/v1".to_owned(),
            source,
            registry: put(&root, &registry.canonical_bytes()),
            configuration,
            data: data_object.clone(),
            calendar,
            environment,
            family: Some(family),
        };
        let context_object = put_json(&root, &context);
        let backtest = put_json(
            &root,
            &model::Backtest {
                schema: "loop.backtest-spec/v1".to_owned(),
                backtest_id: "backtest.manifest".to_owned(),
                context: context_object.clone(),
                factor: factors[0].1.clone(),
                engine: model::Engine::PrimaryCrossSectional,
                engine_version: "synthetic-producer.1".to_owned(),
                sample,
                return_definition: "simple_nav_return".to_owned(),
                deterministic_seed: format!("sha256:{}", "09".repeat(32)),
            },
        );
        let series = |name: &str| {
            artifact(
                &root,
                &format!("loop.synthetic_{name}"),
                "text/csv",
                format!("{name}\n0\n").as_bytes(),
                &[name],
            )
        };
        let result = model::ResultManifest {
            schema: "loop.backtest-result/v1".to_owned(),
            job_id: "job.1".to_owned(),
            specification: backtest.clone(),
            engine: model::Engine::PrimaryCrossSectional,
            engine_version: "synthetic-producer.1".to_owned(),
            metrics: vec![model::Metric {
                name: "net_sharpe".to_owned(),
                value: "1.25".to_owned(),
                unit: "dimensionless".to_owned(),
                estimator: "sample_std_ddof1_sqrt252_zero_rf.v1".to_owned(),
            }],
            artifacts: model::Series {
                factor_values: series("factor_values"),
                target_positions: series("target_positions"),
                orders: series("orders"),
                fills: series("fills"),
                nav: series("nav"),
                simple_returns: series("simple_returns"),
                risk_exposures: series("risk_exposures"),
                cost_ledger: series("cost_ledger"),
            },
            completed_at_ms: NOW,
        };
        let result_artifact = artifact(
            &root,
            "loop.backtest_result",
            "application/json",
            &serde_json::to_vec(&result).unwrap(),
            &[],
        );
        let review = model::Review {
            schema: "loop.admission-review/v1".to_owned(),
            job_id: "job.1".to_owned(),
            result: result_artifact.object.clone(),
            factor_spec_id: factors[0].0.clone(),
            policy: policies[8].clone(),
            library_sha256: format!("sha256:{}", hex(&support::library::snapshot(&[]))),
            eligible_observations: 1000,
            valid_observations: 950,
            minimum_coverage_bps: 9500,
            machine_rejection: String::new(),
            semantic_accepted: true,
            replacements: vec![],
        };
        let review_artifact = artifact(
            &root,
            "loop.admission_review",
            "application/json",
            &serde_json::to_vec(&review).unwrap(),
            &[],
        );
        let catalog = model::Catalog {
            schema: "loop.research-catalog/v1".to_owned(),
            contexts: vec![model::ContextEntry {
                context_id: context_object.sha256.clone(),
                manifest: context_object,
            }],
            backtests: vec![model::BacktestEntry {
                job_id: "job.1".to_owned(),
                specification: backtest,
                result: Some(result_artifact.clone()),
                review: Some(review_artifact),
            }],
        };
        let mut job = support::command(1);
        job.specification.kind = JobKind::Backtest as i32;
        job.specification.input = Some(job_specification::Input::Backtest(BacktestJobInput {
            factor_spec_id: Some(loop_protocol::wire::v1::FactorSpecId {
                value: factors[0].0.clone(),
            }),
            dataset: Some(data.reference(&data_object).unwrap()),
            return_definition: ReturnDefinition::SimpleNavReturn as i32,
            provenance: Some(context.provenance(*registry.identity().as_bytes()).unwrap()),
            deterministic_seed: Some(support::digest(9)),
            budget: Some(support::research::budget()),
        }));
        Self {
            directory,
            root,
            registry,
            catalog,
            context,
            result,
            result_artifact,
            job,
            review,
        }
    }

    pub fn path(&self, reference: &ObjectRef) -> PathBuf {
        self.root.join(&reference.sha256[7..])
    }
    pub fn json<T: Serialize>(&self, value: &T) -> ObjectRef {
        put_json(&self.root, value)
    }
    pub fn context_id(&self) -> String {
        self.catalog.contexts[0].context_id.clone()
    }

    pub fn job_input(&self) -> BacktestJobInput {
        let Some(job_specification::Input::Backtest(input)) = &self.job.specification.input else {
            unreachable!()
        };
        input.clone()
    }

    pub fn success(&self) -> JobSuccess {
        let mut outputs = self
            .result
            .artifacts
            .entries()
            .into_iter()
            .map(|artifact| artifact.wire().unwrap())
            .collect::<Vec<_>>();
        outputs.push(self.result_artifact.wire().unwrap());
        JobSuccess { outputs }
    }

    pub fn republish_result(&mut self) {
        self.result_artifact = artifact(
            &self.root,
            "loop.backtest_result",
            "application/json",
            &serde_json::to_vec(&self.result).unwrap(),
            &[],
        );
        self.review.result = self.result_artifact.object.clone();
        self.catalog.backtests[0].result = Some(self.result_artifact.clone());
        self.catalog.backtests[0].review = Some(artifact(
            &self.root,
            "loop.admission_review",
            "application/json",
            &serde_json::to_vec(&self.review).unwrap(),
            &[],
        ));
    }

    pub fn replace_context(&mut self, context: model::Context) {
        let reference = self.json(&context);
        let mut specification: model::Backtest = serde_json::from_slice(
            &std::fs::read(self.path(&self.catalog.backtests[0].specification)).unwrap(),
        )
        .unwrap();
        let data: model::Dataset =
            serde_json::from_slice(&std::fs::read(self.path(&context.data)).unwrap()).unwrap();
        specification.context = reference.clone();
        specification.sample = data.sample.clone();
        self.catalog.backtests[0].specification = self.json(&specification);
        self.result.specification = self.catalog.backtests[0].specification.clone();
        self.catalog.contexts = vec![model::ContextEntry {
            context_id: reference.sha256.clone(),
            manifest: reference,
        }];
        let dataset = data.reference(&context.data).unwrap();
        let provenance = context
            .provenance(*self.registry.identity().as_bytes())
            .unwrap();
        let Some(job_specification::Input::Backtest(input)) = &mut self.job.specification.input
        else {
            unreachable!()
        };
        input.dataset = Some(dataset);
        input.provenance = Some(provenance);
        self.context = context;
        self.republish_result();
    }

    pub async fn policy(&self) -> Arc<TrustedManifests> {
        Arc::new(
            TrustedManifests::load(
                LocalArtifacts::open(&self.root).unwrap(),
                self.json(&self.catalog),
                vec![self.registry.clone()],
                vec![actor()],
            )
            .await
            .unwrap(),
        )
    }

    pub async fn open(&self, clock: Arc<dyn Clock>, policy: Arc<TrustedManifests>) -> PgJobStore {
        self.open_at(&self.directory.path().join("state"), clock, policy)
            .await
    }

    pub async fn open_at(
        &self,
        state: &Path,
        clock: Arc<dyn Clock>,
        policy: Arc<TrustedManifests>,
    ) -> PgJobStore {
        let mut options = support::base_options(state);
        options.clock = clock;
        options.admission = Arc::new(Authority(self.job.specification.clone()));
        options.backtest_policy = policy;
        PgJobStore::open(options).await.unwrap()
    }
}

struct Authority(JobSpecification);

impl AdmissionPolicy for Authority {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if job == &self.0 {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
    fn authorize_job_command(
        &self,
        _: &str,
        principal: &Actor,
        record: &JobRecord,
    ) -> StoreResult<()> {
        if principal == &actor() && record.specification.as_ref() == Some(&self.0) {
            Ok(())
        } else {
            Err(StoreError::AdmissionDenied)
        }
    }
}

pub(super) fn put(root: &Path, bytes: &[u8]) -> ObjectRef {
    let hex = format!("{:x}", Sha256::digest(bytes));
    let path = root.join(&hex);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => file.write_all(bytes).unwrap(),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            assert_eq!(std::fs::read(path).unwrap(), bytes)
        }
        Err(error) => panic!("fixture object: {error}"),
    }
    ObjectRef {
        sha256: format!("sha256:{hex}"),
        byte_size: bytes.len() as u64,
    }
}

fn put_json<T: Serialize>(root: &Path, value: &T) -> ObjectRef {
    put(root, &serde_json::to_vec(value).unwrap())
}

fn artifact(
    root: &Path,
    name: &str,
    media_type: &str,
    bytes: &[u8],
    columns: &[&str],
) -> model::Artifact {
    let schema = put_json(
        root,
        &model::SchemaDocument {
            schema: "loop.artifact-schema/v1".to_owned(),
            name: name.to_owned(),
            version: 1,
            media_type: media_type.to_owned(),
            columns: columns.iter().map(|column| (*column).to_owned()).collect(),
        },
    );
    model::Artifact {
        object: put(root, bytes),
        schema: model::SchemaRef {
            name: name.to_owned(),
            version: 1,
            document: schema,
        },
        media_type: media_type.to_owned(),
        created_at_ms: NOW,
    }
}

pub(super) fn registry(extra_field: bool) -> OperatorPolicyRegistry {
    let contract = OperatorSemanticContract::new(Identifier::new("mean").unwrap(), PositiveInteger::new("1").unwrap(),
        NullPolicy::IgnoreMissing, WindowPolicy::TrailingArgument2MinimumValidMinNMax3Floor2NDiv3RightInclusiveConstantPreserve,
        TiePolicy::NotApplicable, AlignmentPolicy::UnaryPreserveTimestampAndSecurity, NumericPolicy::Binary64NonFiniteToMissing);
    let mut registry = OperatorRegistryBuilder::new();
    registry
        .register_field(Identifier::new("market.close").unwrap(), ValueType::Series)
        .unwrap();
    if extra_field {
        registry
            .register_field(Identifier::new("market.volume").unwrap(), ValueType::Series)
            .unwrap();
    }
    registry
        .register_operator(
            OperatorDefinition::fixed(
                OperatorRef::new(
                    Identifier::new("mean").unwrap(),
                    PositiveInteger::new("1").unwrap(),
                ),
                vec![
                    ArgumentDefinition::series(),
                    ArgumentDefinition::decimal_literal(
                        DecimalConstraints::new(
                            4,
                            0,
                            CanonicalDecimal::new("1").unwrap(),
                            CanonicalDecimal::new("4096").unwrap(),
                        )
                        .unwrap(),
                    ),
                ],
                ValueType::Series,
                OperatorPolicy::ORDERED,
                contract.identity(),
            )
            .unwrap(),
        )
        .unwrap();
    registry
        .build(&BTreeMap::from([(
            contract.identity(),
            contract.canonical_bytes(),
        )]))
        .unwrap()
}

fn factor(
    root: &Path,
    registry: &OperatorPolicyRegistry,
    policies: &[model::Policy; 9],
    window: u32,
) -> (String, ObjectRef) {
    let expression = FactorExpr::Call(OperatorCall::new(
        OperatorRef::new(
            Identifier::new("mean").unwrap(),
            PositiveInteger::new("1").unwrap(),
        ),
        vec![
            FactorExpr::Field(FieldRef::new(Identifier::new("market.close").unwrap())),
            FactorExpr::Literal(Literal::Decimal(
                CanonicalDecimal::new(window.to_string()).unwrap(),
            )),
        ],
    ));
    let canonical =
        canonical_expression_bytes(&expression, registry, ValidationLimits::default()).unwrap();
    let policy = |index: usize| {
        PolicyRef::new(
            PolicyId::new(&policies[index].policy_id).unwrap(),
            PositiveInteger::new("1").unwrap(),
            policies[index].document.digest().unwrap(),
        )
    };
    let draft = FactorSpecDraft::new(
        expression_id(&expression, registry, ValidationLimits::default()).unwrap(),
        *registry.identity().as_bytes(),
        FactorDirection::HigherIsBetter,
        policy(0),
        policy(1),
        policy(2),
        policy(3),
        policy(4),
        policy(5),
        policy(6),
        policy(7),
        policy(8),
    );
    let factor =
        bind_factor_spec(draft, &canonical, registry, ValidationLimits::default()).unwrap();
    let id = factor_spec_id(&factor).to_string();
    let reference = put_json(
        root,
        &model::Factor {
            schema: "loop.factor-manifest/v1".to_owned(),
            factor_spec_id: id.clone(),
            specification: put(root, &canonical_factor_spec_bytes(&factor)),
            expression: put(root, &canonical),
        },
    );
    (id, reference)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
