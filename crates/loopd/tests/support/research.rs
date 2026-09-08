use loop_protocol::negotiation::{ProtocolBuildIdentity, validate_protocol_selection_availability};
use loop_protocol::wire::research::v1 as research;
use loop_protocol::wire::v1::*;
use loopd::store::{AdmissionPolicy, RoleCommand, StoreError, StoreResult, SubmissionMetadata};

use super::{FixtureAdmission, NOW, actor, context, digest, protocol_info, timestamp};

const FACTOR_ID: &str = "sha256:3a4f28e6e918ec379af280264bf314eaac843a0aded6fefcdcdd06cf925b895a";
const EXPRESSION_ID: &str =
    "sha256:2b93fad0265af4e02df2b2dce69d3b5a221bfd553aacaa52bb652666374d7cb3";

pub struct Admission;

impl AdmissionPolicy for Admission {
    fn validate_submission(&self, job: &JobSpecification) -> StoreResult<()> {
        if job.kind == JobKind::Report as i32 {
            return FixtureAdmission.validate_submission(job);
        }
        if job.submitted_by.as_ref() != Some(&actor())
            || !inputs()
                .iter()
                .any(|(kind, input)| job.kind == *kind as i32 && job.input.as_ref() == Some(input))
        {
            return Err(StoreError::AdmissionDenied);
        }
        validate_protocol_selection_availability(
            job.protocol_selection
                .as_ref()
                .ok_or(StoreError::AdmissionDenied)?,
            &protocol_info(),
            &[ProtocolBuildIdentity {
                build_version: "fixture.1".to_owned(),
                build_sha256: [23; 32],
            }],
            &[[22; 32]],
            "loop.v1",
            &[
                "jobs.envelope.v1",
                "jobs.kind-input.v1",
                "jobs.prelease-terminal.v1",
            ],
        )
        .map_err(|_| StoreError::AdmissionDenied)
    }
}

pub fn command(key: &str) -> RoleCommand {
    let (_, job_specification::Input::Reconciliation(input)) = inputs().pop().unwrap() else {
        unreachable!()
    };
    let budget = input.budget.unwrap();
    RoleCommand::Reconciliation(research::EnqueueReconciliationRequest {
        context: Some(context(key)),
        input: Some(research::ReconciliationInput {
            primary_backtest_id: input.primary_backtest_id,
            independent_backtest_id: input.independent_backtest_id,
            reconciliation_policy: input.reconciliation_policy,
            budget: Some(research::ResearchJobBudget {
                maximum_steps: budget.maximum_steps,
                maximum_input_tokens: budget.maximum_input_tokens,
                maximum_output_tokens: budget.maximum_output_tokens,
                maximum_cost: budget.maximum_cost,
                maximum_wall_time: budget.maximum_wall_time,
            }),
        }),
    })
}

pub fn metadata() -> SubmissionMetadata {
    let source = super::command(1).specification;
    SubmissionMetadata {
        run_id: source.run_id.unwrap(),
        protocol_selection: source.protocol_selection.unwrap(),
    }
}

pub fn inputs() -> Vec<(JobKind, job_specification::Input)> {
    vec![
        (
            JobKind::Discovery,
            job_specification::Input::Discovery(DiscoveryJobInput {
                dataset: Some(dataset()),
                research_policy: Some(policy("policy.research")),
                maker_model: Some(model("resolution.maker")),
                checker_model: Some(model("resolution.checker")),
                budget: Some(budget()),
                maximum_candidates: 37,
            }),
        ),
        (
            JobKind::FactorEvaluation,
            job_specification::Input::FactorEvaluation(FactorEvaluationJobInput {
                factor: Some(factor()),
                dataset: Some(dataset()),
                budget: Some(budget()),
            }),
        ),
        (
            JobKind::Backtest,
            job_specification::Input::Backtest(BacktestJobInput {
                factor_spec_id: Some(FactorSpecId {
                    value: FACTOR_ID.to_owned(),
                }),
                dataset: Some(dataset()),
                return_definition: ReturnDefinition::SimpleNavReturn as i32,
                provenance: Some(ResearchProvenanceFingerprint {
                    source_code_sha256: Some(digest(1)),
                    operator_registry_sha256: Some(digest(2)),
                    configuration_sha256: Some(digest(3)),
                    data_manifest_sha256: Some(digest(7)),
                    trading_calendar_sha256: Some(digest(5)),
                    environment_sha256: Some(digest(6)),
                }),
                deterministic_seed: Some(digest(9)),
                budget: Some(budget()),
            }),
        ),
        (
            JobKind::IndependentReconciliation,
            job_specification::Input::Reconciliation(ReconciliationJobInput {
                primary_backtest_id: Some(BacktestId {
                    value: "backtest.primary".to_owned(),
                }),
                independent_backtest_id: Some(BacktestId {
                    value: "backtest.independent".to_owned(),
                }),
                reconciliation_policy: Some(policy("policy.fixture")),
                budget: Some(budget()),
            }),
        ),
    ]
}

pub fn budget() -> JobBudget {
    let Some(job_specification::Input::Artifact(input)) = super::command(1).specification.input
    else {
        unreachable!()
    };
    input.budget.unwrap()
}

fn dataset() -> DevelopmentDatasetReference {
    DevelopmentDatasetReference {
        snapshot_ids: vec![SnapshotId {
            value: "snapshot.development".to_owned(),
        }],
        manifest_sha256: Some(digest(7)),
    }
}

fn policy(value: &str) -> PolicyReference {
    PolicyReference {
        policy_id: Some(PolicyId {
            value: value.to_owned(),
        }),
        revision: "1".to_owned(),
        sha256: Some(digest(6)),
    }
}

fn factor() -> FactorSpec {
    let expression_id = FactorExpressionId {
        value: EXPRESSION_ID.to_owned(),
    };
    FactorSpec {
        factor_spec_id: Some(FactorSpecId {
            value: FACTOR_ID.to_owned(),
        }),
        expression_id: Some(expression_id.clone()),
        expression: Some(CanonicalFactorAst {
            expression_id: Some(expression_id),
            ast: Some(FactorAst {
                schema_version: 1,
                root: Some(FactorAstNode {
                    node: Some(factor_ast_node::Node::Field(FieldReference {
                        field: "market.close".to_owned(),
                    })),
                }),
            }),
            canonicalization_profile: "loop.factor-ast/v1".to_owned(),
            canonical_json: br#"{"node":"field","field":"market.close"}"#.to_vec(),
        }),
        direction: FactorDirection::HigherIsBetter as i32,
        frozen_policy: Some(FrozenResearchPolicyReference {
            universe_policy: Some(policy("policy.universe")),
            data_policy: Some(policy("policy.data")),
            calendar_policy: Some(policy("policy.calendar")),
            preprocess_policy: Some(policy("policy.preprocess")),
            neutralization_policy: Some(policy("policy.neutralization")),
            portfolio_policy: Some(policy("policy.portfolio")),
            execution_policy: Some(policy("policy.execution")),
            cost_policy: Some(policy("policy.cost")),
            evaluation_policy: Some(policy("policy.evaluation")),
        }),
        operator_registry_sha256: Some(digest(2)),
    }
}

fn model(resolution: &str) -> ModelResolutionSnapshot {
    ModelResolutionSnapshot {
        resolution_id: Some(ModelResolutionId {
            value: resolution.to_owned(),
        }),
        provider_id: Some(ProviderId {
            value: "provider.fixture".to_owned(),
        }),
        model_id: Some(ModelId {
            value: "model.fixture".to_owned(),
        }),
        requested_alias: "fixture-model".to_owned(),
        protocol_family: ModelProtocolFamily::OpenaiResponses as i32,
        capabilities: Some(ModelCapabilities {
            context_window_tokens: 100_000,
            maximum_output_tokens: 20_000,
            ..Default::default()
        }),
        pricing: Some(ModelPricing {
            input_per_million_tokens: Some(money("1")),
            output_per_million_tokens: Some(money("2")),
            cached_input_per_million_tokens: Some(money("0.5")),
        }),
        capability_sha256: Some(digest(11)),
        catalog_sha256: Some(digest(12)),
        resolved_at: Some(timestamp(NOW - 5_000)),
        provider_plugin_name: "fixture-provider".to_owned(),
        provider_plugin_version: "1.0.0".to_owned(),
        provider_plugin_sha256: Some(digest(13)),
        snapshot_sha256: Some(digest(14)),
    }
}

fn money(amount: &str) -> Money {
    Money {
        amount: Some(ExactDecimal {
            value: amount.to_owned(),
        }),
        currency_code: "USD".to_owned(),
    }
}
