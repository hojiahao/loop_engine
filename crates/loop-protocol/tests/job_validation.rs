use std::collections::BTreeMap;

use loop_protocol::job::{
    canonical_protocol_selection_bytes, factor_spec_identity_sha256, protocol_selection_sha256,
    validate_holdout_backtest_plan_entry_binding, validate_job_record, validate_job_specification,
};
use loop_protocol::runtime_validation::validate_job_wire_dispatch_candidate;
use loop_protocol::wire::v1::{
    Actor, ActorId, ActorKind, ArtifactId, ArtifactJobInput, ArtifactRef, ArtifactSchemaReference,
    BacktestId, BacktestJobInput, BacktestSpec, BudgetExhaustion, CanonicalFactorAst, CausationId,
    CivilDate, CorrelationId, DevelopmentDatasetReference, DiscoveryJobInput, ErrorCategory,
    ErrorDetail, ExactDecimal, FactorAst, FactorAstNode, FactorDirection, FactorEvaluationJobInput,
    FactorExpressionId, FactorRejection, FactorRejectionCode, FactorSpec, FactorSpecId,
    FieldReference, FrozenResearchPolicyReference, HoldoutBacktestJobInput,
    HoldoutEvaluationPlanId, HoldoutGrantId, HoldoutGrantReference, HoldoutPeriodId,
    IdempotencyKey, InfrastructureFailure, JobBatchId, JobBudget, JobCancellation, JobId, JobKind,
    JobLease, JobOutcome, JobRecord, JobSpecification, JobState, JobSuccess, LeaseId,
    ModelCapabilities, ModelId, ModelPricing, ModelProtocolFamily, ModelResolutionId,
    ModelResolutionSnapshot, Money, PolicyId, PolicyReference, ProtocolLimits,
    ProtocolSelectionSnapshot, ProviderId, ReconciliationJobInput, ResearchProvenanceFingerprint,
    ReturnDefinition, RunId, SampleRole, SampleWindow, ServiceError, Sha256Digest, SnapshotId,
    factor_ast_node, job_outcome, job_specification,
};
use prost_types::{Duration, Timestamp};

const VECTORS: &str = include_str!("../../../tests/contracts/job_record_vectors.tsv");
const PROTOCOL_GOLDEN: &str =
    include_str!("../../../tests/contracts/protocol_selection_golden.tsv");
const PROTOCOL_NEGATIVE: &str =
    include_str!("../../../tests/contracts/protocol_selection_negative.tsv");
const HOLDOUT_BINDING: &str =
    include_str!("../../../tests/contracts/holdout_job_binding_vectors.tsv");
const HOLDOUT_GRANT_LIFETIME: &str =
    include_str!("../../../tests/contracts/holdout_grant_lifetime_vectors.tsv");
const HOLDOUT_GOLDEN: &str = include_str!("../../../tests/contracts/holdout_identity_golden.json");
const FACTOR_ID: &str = "sha256:3a4f28e6e918ec379af280264bf314eaac843a0aded6fefcdcdd06cf925b895a";
const EXPRESSION_ID: &str =
    "sha256:2b93fad0265af4e02df2b2dce69d3b5a221bfd553aacaa52bb652666374d7cb3";
const OTHER_FACTOR_ID: &str =
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Debug)]
struct Vector<'a> {
    name: &'a str,
    expected: &'a str,
    kind: &'a str,
    input: &'a str,
    state: &'a str,
    lease: &'a str,
    outcome: &'a str,
    attempt: u32,
    mutation: &'a str,
}

#[test]
fn shared_job_record_matrix_fails_closed() {
    let vectors = vectors().collect::<Vec<_>>();
    assert_eq!(vectors.len(), 105, "all shared rows must execute");
    for vector in vectors {
        let record = record(&vector);
        match validate_job_record(&record) {
            Ok(_) => assert_eq!(vector.expected, "accept", "{}", vector.name),
            Err(error) => assert_eq!(error.code.as_str(), vector.expected, "{}", vector.name),
        }
    }
}

#[test]
fn protocol_selection_producer_matches_shared_golden() {
    let fields: Vec<_> = PROTOCOL_GOLDEN
        .lines()
        .nth(1)
        .unwrap()
        .split('\t')
        .collect();
    assert_eq!(fields.len(), 3);
    let selection = protocol_selection();
    assert_eq!(
        canonical_protocol_selection_bytes(&selection).unwrap(),
        fields[1].as_bytes()
    );
    assert_eq!(
        hex(&protocol_selection_sha256(&selection).unwrap()),
        fields[2]
    );
}

#[test]
fn protocol_selection_shared_negatives_fail_closed() {
    for line in PROTOCOL_NEGATIVE.lines().skip(1) {
        let fields: Vec<_> = line.split('\t').collect();
        let vector = Vector {
            name: fields[0],
            expected: fields[1],
            kind: "discovery",
            input: "discovery",
            state: "queued",
            lease: "absent",
            outcome: "absent",
            attempt: 0,
            mutation: "none",
        };
        let mut specification = valid_specification(&vector);
        let selection = specification.protocol_selection.as_mut().unwrap();
        match fields[2] {
            "digest_mismatch" => selection.selection_sha256.as_mut().unwrap().value[0] ^= 0xff,
            "unsorted_features" => selection.enabled_features.swap(0, 1),
            "duplicate_features" => {
                selection.enabled_features[1] = selection.enabled_features[0].clone()
            }
            "zero_limit" => {
                selection
                    .effective_limits
                    .as_mut()
                    .unwrap()
                    .maximum_ast_nodes = 0
            }
            "future_timestamp" => {
                selection.selected_at = Some(timestamp(6));
                selection.selection_sha256 = Some(Sha256Digest {
                    value: protocol_selection_sha256(selection).unwrap().to_vec(),
                });
            }
            "invalid_package" => selection.selected_package = "Loop.v1".to_owned(),
            "zero_package_version" => selection.selected_package = "loop.v0".to_owned(),
            "leading_zero_package_version" => selection.selected_package = "loop.v01".to_owned(),
            other => panic!("unknown selection mutation {other}"),
        }
        let error = validate_job_specification(&specification).unwrap_err();
        assert_eq!(error.code.as_str(), fields[1], "{}", fields[0]);
    }
}

#[test]
fn holdout_job_shared_plan_entry_bindings_fail_closed() {
    for line in HOLDOUT_BINDING.lines().skip(1) {
        let fields: Vec<_> = line.split('\t').collect();
        let mut fixture = holdout_binding_fixture();
        match fields[2] {
            "none" => {}
            "plan_identity" => {
                let identity = HoldoutEvaluationPlanId {
                    value: digest_id(99),
                };
                fixture.input.holdout_evaluation_plan_id = Some(identity.clone());
                fixture
                    .input
                    .consumed_grant
                    .as_mut()
                    .unwrap()
                    .holdout_evaluation_plan_id = Some(identity);
            }
            "entry_factor" => {
                fixture
                    .input
                    .frozen_backtest_spec
                    .as_mut()
                    .unwrap()
                    .factor_spec_id = Some(factor_id(OTHER_FACTOR_ID));
            }
            "entry_budget" => fixture.input.budget.as_mut().unwrap().maximum_steps = 41,
            "entry_artifact" => {
                fixture
                    .input
                    .frozen_backtest_spec
                    .as_mut()
                    .unwrap()
                    .canonical_spec_sha256 = Some(digest(99));
            }
            "canonical_plan_tampered" => fixture.canonical_plan_bytes.push(b' '),
            "canonical_period_tampered" => fixture.canonical_period_bytes.push(b' '),
            other => panic!("unknown holdout binding mutation {other}"),
        }
        match validate_holdout_backtest_plan_entry_binding(
            &fixture.input,
            &timestamp(5),
            &fixture.canonical_period_bytes,
            &fixture.canonical_plan_bytes,
            &fixture.trusted_backtest_schema_sha256,
            &fixture.resolved_backtest_artifacts,
        ) {
            Ok(()) => assert_eq!(fields[1], "accept", "{}", fields[0]),
            Err(error) => assert_eq!(error.code.as_str(), fields[1], "{}", fields[0]),
        }
    }
}

#[test]
fn holdout_grant_lifetime_boundaries_cover_binder_and_wire_candidate() {
    let rows = HOLDOUT_GRANT_LIFETIME.lines().skip(1).collect::<Vec<_>>();
    assert_eq!(rows.len(), 6, "all shared grant lifetime rows must execute");
    for line in rows {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 5, "invalid fixture row: {line}");
        let submitted_at = Timestamp {
            seconds: fields[3].parse().unwrap(),
            nanos: fields[4].parse().unwrap(),
        };
        let fixture = holdout_binding_fixture();
        match validate_holdout_backtest_plan_entry_binding(
            &fixture.input,
            &submitted_at,
            &fixture.canonical_period_bytes,
            &fixture.canonical_plan_bytes,
            &fixture.trusted_backtest_schema_sha256,
            &fixture.resolved_backtest_artifacts,
        ) {
            Ok(()) => assert_eq!(fields[1], "accept", "{} binder", fields[0]),
            Err(error) => assert_eq!(error.code.as_str(), fields[1], "{} binder", fields[0]),
        }

        let vector = Vector {
            name: fields[0],
            expected: "accept",
            kind: "holdout_backtest",
            input: "holdout_backtest",
            state: "queued",
            lease: "absent",
            outcome: "absent",
            attempt: 0,
            mutation: "none",
        };
        let mut specification = valid_specification(&vector);
        specification.submitted_at = Some(submitted_at);
        match validate_job_wire_dispatch_candidate(&specification, &[JobKind::HoldoutBacktest]) {
            Ok(kind) => {
                assert_eq!(fields[2], "accept", "{} wire candidate", fields[0]);
                assert_eq!(kind, JobKind::HoldoutBacktest, "{}", fields[0]);
            }
            Err(error) => assert_eq!(
                error.code.as_str(),
                fields[2],
                "{} wire candidate",
                fields[0]
            ),
        }
    }
}

fn vectors() -> impl Iterator<Item = Vector<'static>> {
    VECTORS.lines().skip(1).map(|line| {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 9, "invalid fixture row: {line}");
        Vector {
            name: fields[0],
            expected: fields[1],
            kind: fields[2],
            input: fields[3],
            state: fields[4],
            lease: fields[5],
            outcome: fields[6],
            attempt: fields[7].parse().unwrap(),
            mutation: fields[8],
        }
    })
}

fn record(vector: &Vector<'_>) -> JobRecord {
    let specification = (vector.kind != "missing").then(|| valid_specification(vector));
    let enforced_budget = specification
        .as_ref()
        .and_then(specification_budget)
        .cloned();
    let active_lease = (vector.lease == "present").then(valid_lease);
    let mut record = JobRecord {
        specification,
        state: state(vector.state),
        revision: 1,
        attempt: vector.attempt,
        active_lease,
        outcome: outcome(vector.outcome, vector.attempt, enforced_budget),
        updated_at: Some(timestamp(20)),
    };
    mutate(&mut record, vector.mutation);
    record
}

fn valid_specification(vector: &Vector<'_>) -> JobSpecification {
    let mut specification = JobSpecification {
        job_id: Some(job_id("job.01")),
        run_id: Some(RunId {
            value: "run.01".to_owned(),
        }),
        kind: kind(vector.kind),
        input: input(vector.input),
        submitted_at: Some(timestamp(5)),
        submitted_by: Some(actor()),
        idempotency_key: Some(IdempotencyKey {
            value: "idem.01".to_owned(),
        }),
        correlation_id: Some(CorrelationId {
            value: "corr.01".to_owned(),
        }),
        causation_id: Some(CausationId {
            value: "cause.01".to_owned(),
        }),
        protocol_selection: Some(protocol_selection()),
    };
    let selection = specification.protocol_selection.as_mut().unwrap();
    selection.selection_sha256 = Some(Sha256Digest {
        value: protocol_selection_sha256(selection).unwrap().to_vec(),
    });
    specification
}

fn mutate(record: &mut JobRecord, mutation: &str) {
    match mutation {
        "none" => {}
        "zero_revision" => record.revision = 0,
        "missing_spec_job_id" => record.specification.as_mut().unwrap().job_id = None,
        "malformed_spec_job_id" => {
            record.specification.as_mut().unwrap().job_id = Some(job_id("bad id"))
        }
        "lease_job_mismatch" => {
            record.active_lease.as_mut().unwrap().job_id = Some(job_id("job.02"))
        }
        "lease_future_revision" => record.active_lease.as_mut().unwrap().acquired_revision = 2,
        "lease_zero_revision" => record.active_lease.as_mut().unwrap().acquired_revision = 0,
        "lease_missing_id" => record.active_lease.as_mut().unwrap().lease_id = None,
        "lease_missing_job_id" => record.active_lease.as_mut().unwrap().job_id = None,
        "lease_missing_owner" => record.active_lease.as_mut().unwrap().owner = None,
        "lease_unknown_owner_kind" => {
            record
                .active_lease
                .as_mut()
                .unwrap()
                .owner
                .as_mut()
                .unwrap()
                .kind = 999
        }
        "lease_missing_timestamp" => record.active_lease.as_mut().unwrap().heartbeat_at = None,
        "lease_invalid_order" => {
            record.active_lease.as_mut().unwrap().heartbeat_at = Some(timestamp(30))
        }
        "factor_input_id_missing" => set_input_factor_id(record, None),
        "factor_input_id_malformed" => set_input_factor_id(record, Some("SHA256:bad")),
        "rejection_id_missing" => rejection(record).factor_spec_id = None,
        "rejection_id_malformed" => {
            rejection(record).factor_spec_id = Some(factor_id("SHA256:bad"))
        }
        "rejection_id_mismatch" => {
            rejection(record).factor_spec_id = Some(factor_id(OTHER_FACTOR_ID))
        }
        "rejection_unspecified_code" => {
            rejection(record).code = FactorRejectionCode::Unspecified as i32
        }
        "rejection_unknown_code" => rejection(record).code = 999,
        "rejection_missing_reason" => rejection(record).reason = " ".to_owned(),
        "rejection_control_reason" => rejection(record).reason = "forged\nrecord".to_owned(),
        "rejection_missing_timestamp" => rejection(record).rejected_at = None,
        "rejection_invalid_timestamp" => {
            rejection(record).rejected_at = Some(Timestamp {
                seconds: 0,
                nanos: 1_000_000_000,
            })
        }
        "rejection_too_many_evidence" => {
            rejection(record).evidence = vec![ArtifactRef::default(); 65]
        }
        "success_too_many_outputs" => success(record).outputs = vec![ArtifactRef::default(); 65],
        "infra_missing_error" => infrastructure(record).error = None,
        "infra_unspecified_category" => {
            infrastructure(record).error.as_mut().unwrap().category =
                ErrorCategory::Unspecified as i32
        }
        "infra_unknown_category" => infrastructure(record).error.as_mut().unwrap().category = 999,
        "infra_missing_code" => infrastructure(record).error.as_mut().unwrap().code.clear(),
        "infra_missing_message" => infrastructure(record)
            .error
            .as_mut()
            .unwrap()
            .message
            .clear(),
        "infra_attempt_mismatch" => infrastructure(record).attempt = 1,
        "infra_missing_timestamp" => infrastructure(record).failed_at = None,
        "infra_too_many_details" => {
            infrastructure(record).error.as_mut().unwrap().details =
                vec![ErrorDetail::default(); 33]
        }
        "infra_invalid_detail" => infrastructure(record)
            .error
            .as_mut()
            .unwrap()
            .details
            .push(ErrorDetail::default()),
        "cancellation_missing_reason" => cancellation(record).reason.clear(),
        "cancellation_missing_actor" => cancellation(record).cancelled_by = None,
        "cancellation_missing_timestamp" => cancellation(record).cancelled_at = None,
        "budget_missing_limit" => budget(record).exhausted_limit.clear(),
        "budget_missing_object" => budget(record).enforced_budget = None,
        "budget_missing_timestamp" => budget(record).exhausted_at = None,
        "lease_predates_submission" => {
            record.active_lease.as_mut().unwrap().issued_at = Some(timestamp(4))
        }
        "lease_expired_at_update" => {
            record.active_lease.as_mut().unwrap().expires_at = Some(timestamp(20))
        }
        "submitted_actor_empty_subject" => record
            .specification
            .as_mut()
            .unwrap()
            .submitted_by
            .as_mut()
            .unwrap()
            .authenticated_subject
            .clear(),
        "submitted_actor_control_subject" => {
            record
                .specification
                .as_mut()
                .unwrap()
                .submitted_by
                .as_mut()
                .unwrap()
                .authenticated_subject = "subject\nforged".to_owned()
        }
        "policy_revision_zero" => discovery_policy(record).revision = "0".to_owned(),
        "policy_revision_leading_zero" => discovery_policy(record).revision = "01".to_owned(),
        "policy_revision_sign" => discovery_policy(record).revision = "+1".to_owned(),
        "factor_expression_hash_mismatch" => factor_spec(record)
            .expression
            .as_mut()
            .unwrap()
            .canonical_json
            .push(b' '),
        "factor_ast_canonical_mismatch" => {
            let root = factor_spec(record)
                .expression
                .as_mut()
                .unwrap()
                .ast
                .as_mut()
                .unwrap()
                .root
                .as_mut()
                .unwrap();
            match root.node.as_mut().unwrap() {
                factor_ast_node::Node::Field(field) => field.field = "market.open".to_owned(),
                _ => panic!("fixture root must be a field"),
            }
        }
        "factor_spec_hash_mismatch" => {
            factor_spec(record).factor_spec_id = Some(factor_id(OTHER_FACTOR_ID))
        }
        "factor_unspecified_direction" => {
            factor_spec(record).direction = FactorDirection::Unspecified as i32
        }
        "protocol_selection_digest_mismatch" => {
            record
                .specification
                .as_mut()
                .unwrap()
                .protocol_selection
                .as_mut()
                .unwrap()
                .selection_sha256
                .as_mut()
                .unwrap()
                .value[0] ^= 0xff
        }
        "holdout_plan_digest_mismatch" => {
            let input = match record
                .specification
                .as_mut()
                .unwrap()
                .input
                .as_mut()
                .unwrap()
            {
                job_specification::Input::HoldoutBacktest(input) => input,
                _ => panic!("mutation requires holdout input"),
            };
            input.evaluation_plan_sha256.as_mut().unwrap().value[0] ^= 0xff;
        }
        "holdout_unknown_sample_role" => {
            let input = match record
                .specification
                .as_mut()
                .unwrap()
                .input
                .as_mut()
                .unwrap()
            {
                job_specification::Input::HoldoutBacktest(input) => input,
                _ => panic!("mutation requires holdout input"),
            };
            input
                .frozen_backtest_spec
                .as_mut()
                .unwrap()
                .sample
                .as_mut()
                .unwrap()
                .role = 999;
        }
        "holdout_unknown_return_definition" => {
            let input = match record
                .specification
                .as_mut()
                .unwrap()
                .input
                .as_mut()
                .unwrap()
            {
                job_specification::Input::HoldoutBacktest(input) => input,
                _ => panic!("mutation requires holdout input"),
            };
            input
                .frozen_backtest_spec
                .as_mut()
                .unwrap()
                .return_definition = 999;
        }
        "model_invalid_plugin_version" => {
            let input = match record
                .specification
                .as_mut()
                .unwrap()
                .input
                .as_mut()
                .unwrap()
            {
                job_specification::Input::Discovery(input) => input,
                _ => panic!("mutation requires discovery input"),
            };
            input.maker_model.as_mut().unwrap().provider_plugin_version = "1 bad".to_owned();
        }
        "dataset_snapshot_id_malformed" => {
            let input = match record
                .specification
                .as_mut()
                .unwrap()
                .input
                .as_mut()
                .unwrap()
            {
                job_specification::Input::Discovery(input) => input,
                _ => panic!("mutation requires discovery input"),
            };
            input.dataset.as_mut().unwrap().snapshot_ids[0].value = "bad id".to_owned();
        }
        "factor_ast_empty_root" => {
            factor_spec(record)
                .expression
                .as_mut()
                .unwrap()
                .ast
                .as_mut()
                .unwrap()
                .root
                .as_mut()
                .unwrap()
                .node = None;
        }
        "holdout_submission_before_grant_issue" => {
            let specification = record.specification.as_mut().unwrap();
            let input = match specification.input.as_mut().unwrap() {
                job_specification::Input::HoldoutBacktest(input) => input,
                _ => panic!("mutation requires holdout input"),
            };
            input.consumed_grant.as_mut().unwrap().issued_at = Some(timestamp(6));
        }
        "holdout_submission_at_grant_expiry" => {
            let specification = record.specification.as_mut().unwrap();
            let input = match specification.input.as_mut().unwrap() {
                job_specification::Input::HoldoutBacktest(input) => input,
                _ => panic!("mutation requires holdout input"),
            };
            input.consumed_grant.as_mut().unwrap().expires_at = Some(timestamp(5));
        }
        "holdout_submission_after_grant_expiry" => {
            let specification = record.specification.as_mut().unwrap();
            specification.submitted_at = Some(timestamp(6));
            let input = match specification.input.as_mut().unwrap() {
                job_specification::Input::HoldoutBacktest(input) => input,
                _ => panic!("mutation requires holdout input"),
            };
            input.consumed_grant.as_mut().unwrap().expires_at = Some(timestamp(5));
        }
        other => panic!("unknown mutation {other}"),
    }
}

fn input(value: &str) -> Option<job_specification::Input> {
    match value {
        "missing" => None,
        "discovery" => Some(job_specification::Input::Discovery(DiscoveryJobInput {
            dataset: Some(development_dataset()),
            research_policy: Some(policy("policy.research")),
            maker_model: Some(model("resolution.maker")),
            checker_model: Some(model("resolution.checker")),
            budget: Some(valid_budget()),
            maximum_candidates: 40,
        })),
        "factor_evaluation" => Some(job_specification::Input::FactorEvaluation(
            FactorEvaluationJobInput {
                factor: Some(valid_factor_spec()),
                dataset: Some(development_dataset()),
                budget: Some(valid_budget()),
            },
        )),
        "backtest" => Some(job_specification::Input::Backtest(BacktestJobInput {
            budget: Some(valid_budget()),
            factor_spec_id: Some(factor_id(FACTOR_ID)),
            dataset: Some(development_dataset()),
            return_definition: ReturnDefinition::SimpleNavReturn as i32,
            provenance: Some(provenance()),
            deterministic_seed: Some(digest(9)),
        })),
        "reconciliation" => Some(job_specification::Input::Reconciliation(
            ReconciliationJobInput {
                primary_backtest_id: Some(BacktestId {
                    value: "backtest.primary".to_owned(),
                }),
                independent_backtest_id: Some(BacktestId {
                    value: "backtest.independent".to_owned(),
                }),
                reconciliation_policy: Some(policy("policy.reconciliation")),
                budget: Some(valid_budget()),
            },
        )),
        "holdout_backtest" => Some(job_specification::Input::HoldoutBacktest(holdout_input())),
        "artifact" => Some(job_specification::Input::Artifact(ArtifactJobInput {
            input: Some(artifact()),
            policy: Some(policy("policy.artifact")),
            budget: Some(valid_budget()),
        })),
        other => panic!("unknown fixture input {other}"),
    }
}

fn outcome(value: &str, attempt: u32, enforced_budget: Option<JobBudget>) -> Option<JobOutcome> {
    let value = match value {
        "absent" => return None,
        "empty" => None,
        "success" => Some(job_outcome::Outcome::Success(JobSuccess::default())),
        "factor_rejection" => Some(job_outcome::Outcome::FactorRejection(FactorRejection {
            factor_spec_id: Some(factor_id(FACTOR_ID)),
            code: FactorRejectionCode::Performance as i32,
            reason: "fails frozen performance threshold".to_owned(),
            rejected_at: Some(timestamp(20)),
            ..Default::default()
        })),
        "infrastructure_failure" => Some(job_outcome::Outcome::InfrastructureFailure(
            InfrastructureFailure {
                error: Some(ServiceError {
                    category: ErrorCategory::Dependency as i32,
                    code: "dataset_unavailable".to_owned(),
                    message: "dataset unavailable".to_owned(),
                    ..Default::default()
                }),
                attempt,
                failed_at: Some(timestamp(20)),
            },
        )),
        "cancellation" => Some(job_outcome::Outcome::Cancellation(JobCancellation {
            reason: "cancelled by operator".to_owned(),
            cancelled_by: Some(actor()),
            cancelled_at: Some(timestamp(20)),
        })),
        "budget_exhaustion" => Some(job_outcome::Outcome::BudgetExhaustion(BudgetExhaustion {
            exhausted_limit: "maximum_steps".to_owned(),
            enforced_budget,
            exhausted_at: Some(timestamp(20)),
        })),
        other => panic!("unknown fixture outcome {other}"),
    };
    Some(JobOutcome { outcome: value })
}

fn valid_lease() -> JobLease {
    JobLease {
        lease_id: Some(LeaseId {
            value: "lease.01".to_owned(),
        }),
        job_id: Some(job_id("job.01")),
        owner: Some(actor()),
        acquired_revision: 1,
        issued_at: Some(timestamp(10)),
        heartbeat_at: Some(timestamp(11)),
        expires_at: Some(timestamp(30)),
    }
}

fn actor() -> Actor {
    Actor {
        actor_id: Some(ActorId {
            value: "worker.01".to_owned(),
        }),
        kind: ActorKind::Service as i32,
        display_name: "Loop worker".to_owned(),
        authenticated_subject: "service:loop-worker".to_owned(),
    }
}

fn valid_budget() -> JobBudget {
    JobBudget {
        maximum_steps: 40,
        maximum_input_tokens: 100_000,
        maximum_output_tokens: 20_000,
        maximum_cost: Some(money("12.5")),
        maximum_wall_time: Some(Duration {
            seconds: 3_600,
            nanos: 0,
        }),
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

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest {
        value: vec![byte; 32],
    }
}

fn digest_id(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
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

fn valid_factor_spec() -> FactorSpec {
    let expression_id = FactorExpressionId {
        value: EXPRESSION_ID.to_owned(),
    };
    let mut factor = FactorSpec {
        factor_spec_id: None,
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
    };
    let computed = factor_spec_identity_sha256(&factor).unwrap();
    assert_eq!(format!("sha256:{}", hex(&computed)), FACTOR_ID);
    factor.factor_spec_id = Some(factor_id(FACTOR_ID));
    factor
}

fn hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn development_dataset() -> DevelopmentDatasetReference {
    DevelopmentDatasetReference {
        snapshot_ids: vec![SnapshotId {
            value: "snapshot.dev.01".to_owned(),
        }],
        manifest_sha256: Some(digest(7)),
    }
}

fn provenance() -> ResearchProvenanceFingerprint {
    ResearchProvenanceFingerprint {
        source_code_sha256: Some(digest(1)),
        operator_registry_sha256: Some(digest(2)),
        configuration_sha256: Some(digest(3)),
        data_manifest_sha256: Some(digest(4)),
        trading_calendar_sha256: Some(digest(5)),
        environment_sha256: Some(digest(6)),
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
        resolved_at: Some(timestamp(1)),
        provider_plugin_name: "fixture-provider".to_owned(),
        provider_plugin_version: "1.0.0".to_owned(),
        provider_plugin_sha256: Some(digest(13)),
        snapshot_sha256: Some(digest(14)),
    }
}

fn protocol_selection() -> ProtocolSelectionSnapshot {
    ProtocolSelectionSnapshot {
        selected_package: "loop.v1".to_owned(),
        enabled_features: vec![
            "jobs.envelope.v1".to_owned(),
            "jobs.kind-input.v1".to_owned(),
        ],
        effective_limits: Some(ProtocolLimits {
            maximum_unary_bytes: 4_194_304,
            maximum_stream_event_bytes: 1_048_576,
            maximum_canonical_ast_bytes: 262_144,
            maximum_ast_nodes: 4_096,
            maximum_ast_depth: 64,
            maximum_page_records: 500,
            maximum_identity_bytes: 128,
            maximum_artifact_uri_bytes: 2_048,
        }),
        server_build_version: "0.2.0-alpha.1".to_owned(),
        server_build_sha256: Some(digest(21)),
        schema_descriptor_sha256: Some(digest(22)),
        selection_sha256: None,
        selected_at: Some(timestamp(2)),
        client_build_version: "0.2.0-alpha.1".to_owned(),
        client_build_sha256: Some(digest(23)),
    }
}

fn artifact() -> ArtifactRef {
    let value = digest_id(24);
    ArtifactRef {
        artifact_id: Some(ArtifactId {
            value: value.clone(),
        }),
        uri: format!("artifact://sha256/{}", &value[7..]),
        sha256: Some(digest(24)),
        schema: Some(ArtifactSchemaReference {
            name: "loop.report".to_owned(),
            version: 1,
            schema_sha256: Some(digest(25)),
        }),
        media_type: "application/json".to_owned(),
        byte_size: 1,
        created_at: Some(timestamp(1)),
        ..Default::default()
    }
}

fn holdout_input() -> HoldoutBacktestJobInput {
    let period_digest = digest(31);
    let plan_id = digest_id(32);
    HoldoutBacktestJobInput {
        consumed_grant: Some(HoldoutGrantReference {
            holdout_grant_id: Some(HoldoutGrantId {
                value: "grant.01".to_owned(),
            }),
            holdout_period_id: Some(HoldoutPeriodId {
                value: digest_id(31),
            }),
            freeze_manifest_sha256: Some(digest(33)),
            issued_at: Some(timestamp(4)),
            expires_at: Some(timestamp(100)),
            holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId {
                value: plan_id.clone(),
            }),
            evaluation_plan_sha256: Some(digest(34)),
            evaluation_plan_entry_count: 2,
            canonical_period_sha256: Some(period_digest),
        }),
        consumed_grant_revision: 1,
        frozen_backtest_spec: Some(BacktestSpec {
            backtest_id: Some(BacktestId {
                value: "backtest.holdout.01".to_owned(),
            }),
            schema_version: 1,
            factor_spec_id: Some(factor_id(FACTOR_ID)),
            snapshot_ids: vec![SnapshotId {
                value: digest_id(35),
            }],
            sample: Some(SampleWindow {
                role: SampleRole::SecondLockedHistoricalHoldout as i32,
                start_inclusive: Some(CivilDate {
                    year: 2025,
                    month: 1,
                    day: 1,
                }),
                end_inclusive: Some(CivilDate {
                    year: 2026,
                    month: 8,
                    day: 31,
                }),
            }),
            return_definition: ReturnDefinition::SimpleNavReturn as i32,
            provenance: Some(provenance()),
            canonical_spec_sha256: Some(digest(36)),
            created_at: Some(timestamp(3)),
            deterministic_seed: Some(digest(37)),
        }),
        budget: Some(valid_budget()),
        job_batch_id: Some(JobBatchId {
            value: "batch.01".to_owned(),
        }),
        holdout_evaluation_plan_id: Some(HoldoutEvaluationPlanId { value: plan_id }),
        evaluation_plan_sha256: Some(digest(34)),
        evaluation_plan_entry_index: 1,
    }
}

struct HoldoutBindingFixture {
    input: HoldoutBacktestJobInput,
    canonical_period_bytes: Vec<u8>,
    canonical_plan_bytes: Vec<u8>,
    trusted_backtest_schema_sha256: [u8; 32],
    resolved_backtest_artifacts: BTreeMap<String, Vec<u8>>,
}

fn holdout_binding_fixture() -> HoldoutBindingFixture {
    let golden: serde_json::Value = serde_json::from_str(HOLDOUT_GOLDEN).unwrap();
    let period = &golden["periods"][0];
    let plan = &golden["plans"][0];
    let plan_value: serde_json::Value =
        serde_json::from_str(plan["canonical_json"].as_str().unwrap()).unwrap();
    let first_entry = &plan_value["entries"][0];
    let mut input = holdout_input();
    let grant = input.consumed_grant.as_mut().unwrap();
    let period_id = period["holdout_period_id"].as_str().unwrap();
    let period_digest = raw_digest(period["canonical_sha256"].as_str().unwrap());
    let plan_id = plan["holdout_evaluation_plan_id"].as_str().unwrap();
    let plan_digest = raw_digest(plan["plan_sha256"].as_str().unwrap());
    grant.holdout_period_id = Some(HoldoutPeriodId {
        value: period_id.to_owned(),
    });
    grant.canonical_period_sha256 = Some(Sha256Digest {
        value: period_digest.to_vec(),
    });
    grant.holdout_evaluation_plan_id = Some(HoldoutEvaluationPlanId {
        value: plan_id.to_owned(),
    });
    grant.evaluation_plan_sha256 = Some(Sha256Digest {
        value: plan_digest.to_vec(),
    });
    grant.evaluation_plan_entry_count = 2;
    input.holdout_evaluation_plan_id = Some(HoldoutEvaluationPlanId {
        value: plan_id.to_owned(),
    });
    input.evaluation_plan_sha256 = Some(Sha256Digest {
        value: plan_digest.to_vec(),
    });
    let frozen = input.frozen_backtest_spec.as_mut().unwrap();
    frozen.factor_spec_id = Some(factor_id(first_entry["factor_spec_id"].as_str().unwrap()));
    frozen.canonical_spec_sha256 = Some(Sha256Digest {
        value: raw_digest(
            first_entry["backtest_spec_artifact"]["sha256"]
                .as_str()
                .unwrap(),
        )
        .to_vec(),
    });
    frozen.sample.as_mut().unwrap().role = SampleRole::FirstLockedConfirmation as i32;
    frozen.sample.as_mut().unwrap().start_inclusive = Some(CivilDate {
        year: 2021,
        month: 1,
        day: 1,
    });
    frozen.sample.as_mut().unwrap().end_inclusive = Some(CivilDate {
        year: 2024,
        month: 12,
        day: 31,
    });
    let resolved_backtest_artifacts = golden["backtest_artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|artifact| {
            (
                artifact["sha256"].as_str().unwrap().to_owned(),
                artifact["content"].as_str().unwrap().as_bytes().to_vec(),
            )
        })
        .collect();
    HoldoutBindingFixture {
        input,
        canonical_period_bytes: period["canonical_json"]
            .as_str()
            .unwrap()
            .as_bytes()
            .to_vec(),
        canonical_plan_bytes: plan["canonical_json"].as_str().unwrap().as_bytes().to_vec(),
        trusted_backtest_schema_sha256: raw_digest(
            golden["trusted_backtest_schema_sha256"].as_str().unwrap(),
        ),
        resolved_backtest_artifacts,
    }
}

fn raw_digest(value: &str) -> [u8; 32] {
    let hex = value.strip_prefix("sha256:").unwrap();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
        .collect::<Vec<_>>();
    bytes.try_into().unwrap()
}

fn specification_budget(specification: &JobSpecification) -> Option<&JobBudget> {
    match specification.input.as_ref()? {
        job_specification::Input::Discovery(value) => value.budget.as_ref(),
        job_specification::Input::FactorEvaluation(value) => value.budget.as_ref(),
        job_specification::Input::Backtest(value) => value.budget.as_ref(),
        job_specification::Input::Reconciliation(value) => value.budget.as_ref(),
        job_specification::Input::HoldoutBacktest(value) => value.budget.as_ref(),
        job_specification::Input::Artifact(value) => value.budget.as_ref(),
    }
}

fn set_input_factor_id(record: &mut JobRecord, value: Option<&str>) {
    let identity = value.map(factor_id);
    match record
        .specification
        .as_mut()
        .unwrap()
        .input
        .as_mut()
        .unwrap()
    {
        job_specification::Input::FactorEvaluation(input) => {
            input.factor.as_mut().unwrap().factor_spec_id = identity
        }
        job_specification::Input::Backtest(input) => input.factor_spec_id = identity,
        job_specification::Input::HoldoutBacktest(input) => {
            input.frozen_backtest_spec.as_mut().unwrap().factor_spec_id = identity
        }
        _ => panic!("mutation requires factor input"),
    }
}

fn rejection(record: &mut JobRecord) -> &mut FactorRejection {
    match record.outcome.as_mut().unwrap().outcome.as_mut().unwrap() {
        job_outcome::Outcome::FactorRejection(value) => value,
        _ => panic!("expected factor rejection"),
    }
}

fn infrastructure(record: &mut JobRecord) -> &mut InfrastructureFailure {
    match record.outcome.as_mut().unwrap().outcome.as_mut().unwrap() {
        job_outcome::Outcome::InfrastructureFailure(value) => value,
        _ => panic!("expected infrastructure failure"),
    }
}

fn success(record: &mut JobRecord) -> &mut JobSuccess {
    match record.outcome.as_mut().unwrap().outcome.as_mut().unwrap() {
        job_outcome::Outcome::Success(value) => value,
        _ => panic!("expected success"),
    }
}

fn cancellation(record: &mut JobRecord) -> &mut JobCancellation {
    match record.outcome.as_mut().unwrap().outcome.as_mut().unwrap() {
        job_outcome::Outcome::Cancellation(value) => value,
        _ => panic!("expected cancellation"),
    }
}

fn budget(record: &mut JobRecord) -> &mut BudgetExhaustion {
    match record.outcome.as_mut().unwrap().outcome.as_mut().unwrap() {
        job_outcome::Outcome::BudgetExhaustion(value) => value,
        _ => panic!("expected budget exhaustion"),
    }
}

fn discovery_policy(record: &mut JobRecord) -> &mut PolicyReference {
    match record
        .specification
        .as_mut()
        .unwrap()
        .input
        .as_mut()
        .unwrap()
    {
        job_specification::Input::Discovery(value) => value.research_policy.as_mut().unwrap(),
        _ => panic!("mutation requires discovery input"),
    }
}

fn factor_spec(record: &mut JobRecord) -> &mut FactorSpec {
    match record
        .specification
        .as_mut()
        .unwrap()
        .input
        .as_mut()
        .unwrap()
    {
        job_specification::Input::FactorEvaluation(value) => value.factor.as_mut().unwrap(),
        _ => panic!("mutation requires factor-evaluation input"),
    }
}

fn job_id(value: &str) -> JobId {
    JobId {
        value: value.to_owned(),
    }
}
fn factor_id(value: &str) -> FactorSpecId {
    FactorSpecId {
        value: value.to_owned(),
    }
}
fn timestamp(seconds: i64) -> Timestamp {
    Timestamp { seconds, nanos: 0 }
}

fn kind(value: &str) -> i32 {
    match value {
        "unspecified" => JobKind::Unspecified as i32,
        "unknown" => 999,
        "discovery" => JobKind::Discovery as i32,
        "factor_evaluation" => JobKind::FactorEvaluation as i32,
        "backtest" => JobKind::Backtest as i32,
        "independent_reconciliation" => JobKind::IndependentReconciliation as i32,
        "report" => JobKind::Report as i32,
        "prospective_observation" => JobKind::ProspectiveObservation as i32,
        "holdout_backtest" => JobKind::HoldoutBacktest as i32,
        other => panic!("unknown fixture kind {other}"),
    }
}

fn state(value: &str) -> i32 {
    match value {
        "unspecified" => JobState::Unspecified as i32,
        "unknown" => 999,
        "queued" => JobState::Queued as i32,
        "leased" => JobState::Leased as i32,
        "running" => JobState::Running as i32,
        "succeeded" => JobState::Succeeded as i32,
        "factor_rejected" => JobState::FactorRejected as i32,
        "infrastructure_failed" => JobState::InfrastructureFailed as i32,
        "cancelled" => JobState::Cancelled as i32,
        "budget_exhausted" => JobState::BudgetExhausted as i32,
        other => panic!("unknown fixture state {other}"),
    }
}
