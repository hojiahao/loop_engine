//! Fail-closed validation for durable job DTOs.
//!
//! Generated Protobuf values can represent contradictory discriminants and
//! state. Every DTO-to-domain boundary must validate the protocol shape before
//! persistence or dispatch. Deployment support is deliberately separate: a
//! structurally valid kind may still have no enabled handler in this process.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sha2::{Digest, Sha256};

use loop_core::factor::{CanonicalDecimal, Identifier, PositiveInteger};
use loop_core::holdout::{parse_canonical_holdout_evaluation_plan, verify_holdout_period_identity};

use crate::artifact::validate_artifact_ref;
use crate::wire::v1::{
    Actor, ActorKind, BacktestSpec, DevelopmentDatasetReference, ErrorCategory, FactorAst,
    FactorAstNode, FactorDirection, FactorRejection, FactorRejectionCode, FactorSpec,
    HoldoutBacktestJobInput, HoldoutGrantReference, InfrastructureFailure, JobBudget,
    JobCancellation, JobKind, JobRecord, JobSpecification, JobState, ModelProtocolFamily,
    ModelResolutionSnapshot, PolicyReference, ProtocolLimits, ProtocolSelectionSnapshot,
    ResearchProvenanceFingerprint, ReturnDefinition, SampleRole, ServiceError, Sha256Digest,
    factor_ast_node, factor_literal, job_outcome, job_specification,
};

const MAX_ID_BYTES: usize = 128;
const MAX_REASON_BYTES: usize = 2_048;
const MAX_ERROR_CODE_BYTES: usize = 128;
const MAX_ERROR_MESSAGE_BYTES: usize = 2_048;
const MAX_ERROR_FIELD_PATH_BYTES: usize = 512;
const MAX_ERROR_DETAILS: usize = 32;
const MAX_OUTCOME_ARTIFACTS: usize = 64;
const MAX_DATASET_SNAPSHOTS: usize = 128;
const MAX_PROTOCOL_FEATURES: usize = 256;
const MAX_PROTOCOL_NAME_BYTES: usize = 128;
const MAX_BUILD_VERSION_BYTES: usize = 128;
const MAX_ACTOR_DISPLAY_NAME_BYTES: usize = 256;
const MAX_AUTHENTICATED_SUBJECT_BYTES: usize = 512;
const MAX_JOB_STEPS: u32 = 1_000_000;
const MAX_JOB_TOKENS: u64 = 1_000_000_000_000;
const MAX_JOB_WALL_TIME_SECONDS: i64 = 604_800;
const MAX_HOLDOUT_ENTRIES: u32 = 4_096;
const MAX_CANDIDATES: u32 = 1_000_000;
const MAX_PROTOCOL_UNARY_BYTES: u64 = 4 * 1_024 * 1_024;
const MAX_PROTOCOL_STREAM_EVENT_BYTES: u64 = 1_024 * 1_024;
const MAX_PROTOCOL_CANONICAL_AST_BYTES: u64 = 256 * 1_024;
const MAX_PROTOCOL_AST_NODES: u32 = 4_096;
const MAX_PROTOCOL_AST_DEPTH: u32 = 64;
const MAX_PROTOCOL_AST_DIRECT_ARGUMENTS: usize = 1_024;
const MAX_PROTOCOL_PAGE_RECORDS: u32 = 500;
const MAX_PROTOCOL_IDENTITY_BYTES: u32 = 128;
const MAX_PROTOCOL_ARTIFACT_URI_BYTES: u32 = 2_048;
const PROTOCOL_SELECTION_DOMAIN: &[u8] = b"loop.protocol-selection/v1";
const FACTOR_AST_DOMAIN: &[u8] = b"loop.factor-ast/v1";
const FACTOR_SPEC_DOMAIN: &[u8] = b"loop.factor-spec/v1";
const MIN_TIMESTAMP_SECONDS: i64 = -62_135_596_800;
const MAX_TIMESTAMP_SECONDS: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobValidationCode {
    MissingField,
    UnknownEnum,
    KindInputMismatch,
    StateLeaseMismatch,
    StateOutcomeMismatch,
    RejectionNotAllowed,
    LeaseJobMismatch,
    InvalidAttempt,
    InvalidRevision,
    InvalidIdentity,
    FactorIdentityMismatch,
    InvalidLease,
    InvalidTerminalPayload,
    CollectionLimit,
    InvalidEnvelope,
    InvalidBudget,
    InvalidProtocolSelection,
    InvalidInput,
    InvalidProvenance,
    BindingMismatch,
}

impl JobValidationCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingField => "missing_field",
            Self::UnknownEnum => "unknown_enum",
            Self::KindInputMismatch => "kind_input_mismatch",
            Self::StateLeaseMismatch => "state_lease_mismatch",
            Self::StateOutcomeMismatch => "state_outcome_mismatch",
            Self::RejectionNotAllowed => "rejection_not_allowed",
            Self::LeaseJobMismatch => "lease_job_mismatch",
            Self::InvalidAttempt => "invalid_attempt",
            Self::InvalidRevision => "invalid_revision",
            Self::InvalidIdentity => "invalid_identity",
            Self::FactorIdentityMismatch => "factor_identity_mismatch",
            Self::InvalidLease => "invalid_lease",
            Self::InvalidTerminalPayload => "invalid_terminal_payload",
            Self::CollectionLimit => "collection_limit",
            Self::InvalidEnvelope => "invalid_envelope",
            Self::InvalidBudget => "invalid_budget",
            Self::InvalidProtocolSelection => "invalid_protocol_selection",
            Self::InvalidInput => "invalid_input",
            Self::InvalidProvenance => "invalid_provenance",
            Self::BindingMismatch => "binding_mismatch",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobValidationError {
    pub code: JobValidationCode,
    pub field: &'static str,
}

impl JobValidationError {
    const fn new(code: JobValidationCode, field: &'static str) -> Self {
        Self { code, field }
    }
}

impl Display for JobValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} failed job validation ({})",
            self.field,
            self.code.as_str()
        )
    }
}

impl Error for JobValidationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedJobSpecificationShape {
    pub kind: JobKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedJobShape {
    pub kind: JobKind,
    pub state: JobState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GrantValidityWindow {
    issued_at: (i64, i32),
    expires_at: (i64, i32),
}

/// Classify the protocol's closed v1 `JobKind`/input matrix.
///
/// This deliberately performs no envelope or input validation. Security
/// boundaries call [`validate_job_specification`]; this narrower classifier is
/// retained so unknown discriminants can be reported without dispatching them.
pub fn validate_job_specification_shape(
    specification: &JobSpecification,
) -> Result<ValidatedJobSpecificationShape, JobValidationError> {
    let kind = JobKind::try_from(specification.kind).map_err(|_| {
        JobValidationError::new(JobValidationCode::UnknownEnum, "specification.kind")
    })?;
    if kind == JobKind::Unspecified {
        return Err(JobValidationError::new(
            JobValidationCode::UnknownEnum,
            "specification.kind",
        ));
    }
    let input = specification.input.as_ref().ok_or_else(|| {
        JobValidationError::new(JobValidationCode::MissingField, "specification.input")
    })?;
    let compatible = matches!(
        (kind, input),
        (JobKind::Discovery, job_specification::Input::Discovery(_))
            | (
                JobKind::FactorEvaluation,
                job_specification::Input::FactorEvaluation(_)
            )
            | (JobKind::Backtest, job_specification::Input::Backtest(_))
            | (
                JobKind::IndependentReconciliation,
                job_specification::Input::Reconciliation(_)
            )
            | (JobKind::Report, job_specification::Input::Artifact(_))
            | (
                JobKind::ProspectiveObservation,
                job_specification::Input::Artifact(_)
            )
            | (
                JobKind::HoldoutBacktest,
                job_specification::Input::HoldoutBacktest(_)
            )
    );
    if !compatible {
        return Err(JobValidationError::new(
            JobValidationCode::KindInputMismatch,
            "specification.input",
        ));
    }
    Ok(ValidatedJobSpecificationShape { kind })
}

/// Validate a v1 job wire envelope and every binding provable from its inline fields.
///
/// For an inline `FactorSpec`, this verifies both content-addressed identities.
/// The wire boundary proves the attached AST and canonical JSON encode one
/// exact tree. Registry resolution, AST typing, and normalization proof remain
/// the factor domain's `bind_factor_spec` responsibility.
/// Development dataset references are opaque identities here; Phase 4/5
/// server-owned snapshot registry and capability resolution must verify their
/// roles before persistence or dispatch.
pub fn validate_job_specification(
    specification: &JobSpecification,
) -> Result<ValidatedJobSpecificationShape, JobValidationError> {
    let shape = validate_job_specification_shape(specification)?;
    let submitted_at = validate_job_envelope(specification)?;
    let input = specification
        .input
        .as_ref()
        .expect("shape validation requires an input");
    validate_job_input(input, submitted_at)?;
    Ok(shape)
}

fn validate_job_envelope(
    specification: &JobSpecification,
) -> Result<(i64, i32), JobValidationError> {
    require_token_id(
        specification
            .job_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.job_id",
    )?;
    require_token_id(
        specification
            .run_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.run_id",
    )?;
    let submitted_at = require_timestamp(
        specification.submitted_at.as_ref(),
        "specification.submitted_at",
    )?;
    validate_actor(
        specification.submitted_by.as_ref(),
        "specification.submitted_by",
    )?;
    require_token_id(
        specification
            .idempotency_key
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.idempotency_key",
    )?;
    require_token_id(
        specification
            .correlation_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.correlation_id",
    )?;
    require_token_id(
        specification
            .causation_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.causation_id",
    )?;
    let selection = specification.protocol_selection.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.protocol_selection",
        )
    })?;
    validate_protocol_selection(selection, submitted_at)?;
    Ok(submitted_at)
}

fn validate_protocol_selection(
    selection: &ProtocolSelectionSnapshot,
    submitted_at: (i64, i32),
) -> Result<(), JobValidationError> {
    let canonical = canonical_protocol_selection_bytes(selection)?;
    let actual = require_digest(
        selection.selection_sha256.as_ref(),
        "specification.protocol_selection.selection_sha256",
    )?;
    let expected = protocol_selection_domain_digest(&canonical);
    if !constant_time_eq(&actual, &expected) {
        return invalid_protocol_selection("specification.protocol_selection.selection_sha256");
    }
    let selected_at = require_timestamp(
        selection.selected_at.as_ref(),
        "specification.protocol_selection.selected_at",
    )?;
    if selected_at > submitted_at {
        return invalid_protocol_selection("specification.protocol_selection.selected_at");
    }
    Ok(())
}

/// Emit the normative canonical v1 protocol-selection document.
///
/// `selection_sha256` is deliberately excluded. Callers that construct a new
/// selection hash the returned bytes with the documented domain separator;
/// ordinary job validation additionally verifies the claimed digest.
pub fn canonical_protocol_selection_bytes(
    selection: &ProtocolSelectionSnapshot,
) -> Result<Vec<u8>, JobValidationError> {
    if !is_protocol_package(&selection.selected_package) {
        return invalid_protocol_selection("specification.protocol_selection.selected_package");
    }
    if selection.enabled_features.len() > MAX_PROTOCOL_FEATURES
        || selection
            .enabled_features
            .iter()
            .any(|feature| !is_protocol_feature(feature))
        || selection
            .enabled_features
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return invalid_protocol_selection("specification.protocol_selection.enabled_features");
    }
    let limits = selection.effective_limits.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.protocol_selection.effective_limits",
        )
    })?;
    validate_protocol_limits(limits)?;
    if !is_build_version(&selection.server_build_version)
        || !is_build_version(&selection.client_build_version)
    {
        return invalid_protocol_selection("specification.protocol_selection.build_version");
    }
    let server_build = require_digest(
        selection.server_build_sha256.as_ref(),
        "specification.protocol_selection.server_build_sha256",
    )?;
    let descriptor = require_digest(
        selection.schema_descriptor_sha256.as_ref(),
        "specification.protocol_selection.schema_descriptor_sha256",
    )?;
    let client_build = require_digest(
        selection.client_build_sha256.as_ref(),
        "specification.protocol_selection.client_build_sha256",
    )?;
    let selected_at = require_timestamp(
        selection.selected_at.as_ref(),
        "specification.protocol_selection.selected_at",
    )?;

    let mut output = String::with_capacity(1_024);
    output.push_str("{\"schema\":\"loop.protocol-selection/v1\",\"selected_package\":\"");
    output.push_str(&selection.selected_package);
    output.push_str("\",\"enabled_features\":[");
    for (index, feature) in selection.enabled_features.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push('"');
        output.push_str(feature);
        output.push('"');
    }
    output.push_str("],\"effective_limits\":{");
    output.push_str(&format!(
        "\"maximum_unary_bytes\":\"{}\",\"maximum_stream_event_bytes\":\"{}\",\"maximum_canonical_ast_bytes\":\"{}\",\"maximum_ast_nodes\":\"{}\",\"maximum_ast_depth\":\"{}\",\"maximum_page_records\":\"{}\",\"maximum_identity_bytes\":\"{}\",\"maximum_artifact_uri_bytes\":\"{}\"",
        limits.maximum_unary_bytes,
        limits.maximum_stream_event_bytes,
        limits.maximum_canonical_ast_bytes,
        limits.maximum_ast_nodes,
        limits.maximum_ast_depth,
        limits.maximum_page_records,
        limits.maximum_identity_bytes,
        limits.maximum_artifact_uri_bytes,
    ));
    output.push_str("},\"server_build_version\":\"");
    output.push_str(&selection.server_build_version);
    output.push_str("\",\"server_build_sha256\":\"");
    output.push_str(&encode_digest(&server_build));
    output.push_str("\",\"schema_descriptor_sha256\":\"");
    output.push_str(&encode_digest(&descriptor));
    output.push_str("\",\"selected_at\":{\"seconds\":\"");
    output.push_str(&selected_at.0.to_string());
    output.push_str("\",\"nanos\":\"");
    output.push_str(&selected_at.1.to_string());
    output.push_str("\"},\"client_build_version\":\"");
    output.push_str(&selection.client_build_version);
    output.push_str("\",\"client_build_sha256\":\"");
    output.push_str(&encode_digest(&client_build));
    output.push_str("\"}");
    Ok(output.into_bytes())
}

/// Compute the domain-separated digest claimed by `selection_sha256`.
pub fn protocol_selection_sha256(
    selection: &ProtocolSelectionSnapshot,
) -> Result<[u8; 32], JobValidationError> {
    canonical_protocol_selection_bytes(selection)
        .map(|bytes| protocol_selection_domain_digest(&bytes))
}

fn protocol_selection_domain_digest(canonical: &[u8]) -> [u8; 32] {
    domain_digest(PROTOCOL_SELECTION_DOMAIN, canonical)
}

fn domain_digest(domain: &[u8], canonical: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0]);
    hasher.update(canonical);
    hasher.finalize().into()
}

fn validate_protocol_limits(limits: &ProtocolLimits) -> Result<(), JobValidationError> {
    let valid = (1..=MAX_PROTOCOL_UNARY_BYTES).contains(&limits.maximum_unary_bytes)
        && (1..=MAX_PROTOCOL_STREAM_EVENT_BYTES).contains(&limits.maximum_stream_event_bytes)
        && (1..=MAX_PROTOCOL_CANONICAL_AST_BYTES).contains(&limits.maximum_canonical_ast_bytes)
        && (1..=MAX_PROTOCOL_AST_NODES).contains(&limits.maximum_ast_nodes)
        && (1..=MAX_PROTOCOL_AST_DEPTH).contains(&limits.maximum_ast_depth)
        && (1..=MAX_PROTOCOL_PAGE_RECORDS).contains(&limits.maximum_page_records)
        && (1..=MAX_PROTOCOL_IDENTITY_BYTES).contains(&limits.maximum_identity_bytes)
        && (1..=MAX_PROTOCOL_ARTIFACT_URI_BYTES).contains(&limits.maximum_artifact_uri_bytes)
        && limits.maximum_canonical_ast_bytes <= limits.maximum_unary_bytes
        && u64::from(limits.maximum_identity_bytes) <= limits.maximum_unary_bytes
        && u64::from(limits.maximum_artifact_uri_bytes) <= limits.maximum_unary_bytes;
    if !valid {
        return invalid_protocol_selection("specification.protocol_selection.effective_limits");
    }
    Ok(())
}

fn validate_job_input(
    input: &job_specification::Input,
    submitted_at: (i64, i32),
) -> Result<(), JobValidationError> {
    match input {
        job_specification::Input::Discovery(value) => {
            validate_development_dataset(value.dataset.as_ref())?;
            validate_policy_reference(
                value.research_policy.as_ref(),
                "specification.input.discovery.research_policy",
            )?;
            validate_model_resolution(
                value.maker_model.as_ref(),
                "specification.input.discovery.maker_model",
                submitted_at,
            )?;
            validate_model_resolution(
                value.checker_model.as_ref(),
                "specification.input.discovery.checker_model",
                submitted_at,
            )?;
            validate_budget(
                value.budget.as_ref(),
                "specification.input.discovery.budget",
            )?;
            if !(1..=MAX_CANDIDATES).contains(&value.maximum_candidates) {
                return invalid_input("specification.input.discovery.maximum_candidates");
            }
            Ok(())
        }
        job_specification::Input::FactorEvaluation(value) => {
            let factor = value.factor.as_ref().ok_or_else(|| {
                JobValidationError::new(
                    JobValidationCode::MissingField,
                    "specification.input.factor_evaluation.factor",
                )
            })?;
            validate_factor_spec_identity_envelope(factor)?;
            validate_development_dataset(value.dataset.as_ref())?;
            validate_budget(
                value.budget.as_ref(),
                "specification.input.factor_evaluation.budget",
            )
        }
        job_specification::Input::Backtest(value) => {
            validate_budget(value.budget.as_ref(), "specification.input.backtest.budget")?;
            require_sha256_id(
                value
                    .factor_spec_id
                    .as_ref()
                    .map(|identity| identity.value.as_str()),
                "specification.input.backtest.factor_spec_id",
            )?;
            validate_development_dataset(value.dataset.as_ref())?;
            validate_simple_return(
                value.return_definition,
                "specification.input.backtest.return_definition",
            )?;
            validate_provenance(
                value.provenance.as_ref(),
                "specification.input.backtest.provenance",
            )?;
            require_digest(
                value.deterministic_seed.as_ref(),
                "specification.input.backtest.deterministic_seed",
            )?;
            Ok(())
        }
        job_specification::Input::Reconciliation(value) => {
            let primary = require_token_id(
                value
                    .primary_backtest_id
                    .as_ref()
                    .map(|identity| identity.value.as_str()),
                "specification.input.reconciliation.primary_backtest_id",
            )?;
            let independent = require_token_id(
                value
                    .independent_backtest_id
                    .as_ref()
                    .map(|identity| identity.value.as_str()),
                "specification.input.reconciliation.independent_backtest_id",
            )?;
            if primary == independent {
                return binding_mismatch("specification.input.reconciliation.backtest_ids");
            }
            validate_policy_reference(
                value.reconciliation_policy.as_ref(),
                "specification.input.reconciliation.reconciliation_policy",
            )?;
            validate_budget(
                value.budget.as_ref(),
                "specification.input.reconciliation.budget",
            )
        }
        job_specification::Input::HoldoutBacktest(value) => {
            validate_holdout_backtest_input(value, submitted_at)
        }
        job_specification::Input::Artifact(value) => {
            let artifact = value.input.as_ref().ok_or_else(|| {
                JobValidationError::new(
                    JobValidationCode::MissingField,
                    "specification.input.artifact.input",
                )
            })?;
            validate_artifact_ref(artifact).map_err(|_| {
                JobValidationError::new(
                    JobValidationCode::InvalidInput,
                    "specification.input.artifact.input",
                )
            })?;
            validate_policy_reference(
                value.policy.as_ref(),
                "specification.input.artifact.policy",
            )?;
            validate_budget(value.budget.as_ref(), "specification.input.artifact.budget")
        }
    }
}

fn validate_development_dataset(
    dataset: Option<&DevelopmentDatasetReference>,
) -> Result<(), JobValidationError> {
    let dataset = dataset.ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.input.dataset",
        )
    })?;
    if dataset.snapshot_ids.is_empty() || dataset.snapshot_ids.len() > MAX_DATASET_SNAPSHOTS {
        return invalid_input("specification.input.dataset.snapshot_ids");
    }
    let mut previous: Option<&str> = None;
    for snapshot in &dataset.snapshot_ids {
        let value = require_token_id(
            Some(snapshot.value.as_str()),
            "specification.input.dataset.snapshot_ids",
        )?;
        if previous.is_some_and(|prior| prior >= value) {
            return invalid_input("specification.input.dataset.snapshot_ids");
        }
        previous = Some(value);
    }
    require_digest(
        dataset.manifest_sha256.as_ref(),
        "specification.input.dataset.manifest_sha256",
    )?;
    Ok(())
}

fn validate_policy_reference(
    policy: Option<&PolicyReference>,
    field: &'static str,
) -> Result<(), JobValidationError> {
    let policy =
        policy.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    let policy_id = policy
        .policy_id
        .as_ref()
        .ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?
        .value
        .as_str();
    if !is_policy_id(policy_id) {
        return invalid_input(field);
    }
    if policy.revision.is_empty()
        || policy.revision.len() > 20
        || policy.revision.starts_with('0')
        || !policy.revision.bytes().all(|byte| byte.is_ascii_digit())
        || policy.revision.parse::<u64>().is_err()
    {
        return invalid_input(field);
    }
    require_digest(policy.sha256.as_ref(), field)?;
    Ok(())
}

/// Emit the canonical FactorSpec identity document after validating every
/// input-local identity invariant. This proves the attached wire AST is the
/// exact tree encoded by `canonical_json`; registry resolution and semantic
/// normalization remain mandatory at the domain binding boundary.
pub fn canonical_factor_spec_identity_bytes(
    factor: &FactorSpec,
) -> Result<Vec<u8>, JobValidationError> {
    let factor_expression_id = require_sha256_id(
        factor
            .expression_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.input.factor_evaluation.factor.expression_id",
    )?;
    let expression = factor.expression.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.input.factor_evaluation.factor.expression",
        )
    })?;
    let attached_expression_id = require_sha256_id(
        expression
            .expression_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.input.factor_evaluation.factor.expression.expression_id",
    )?;
    if !constant_time_eq(
        factor_expression_id.as_bytes(),
        attached_expression_id.as_bytes(),
    ) {
        return binding_mismatch(
            "specification.input.factor_evaluation.factor.expression.expression_id",
        );
    }
    if expression.canonicalization_profile != "loop.factor-ast/v1"
        || expression.canonical_json.is_empty()
        || expression.canonical_json.len() > MAX_PROTOCOL_CANONICAL_AST_BYTES as usize
    {
        return invalid_input("specification.input.factor_evaluation.factor.expression");
    }
    let ast = expression.ast.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.input.factor_evaluation.factor.expression.ast",
        )
    })?;
    if ast.schema_version != 1 || ast.root.as_ref().is_none_or(|root| root.node.is_none()) {
        return invalid_input("specification.input.factor_evaluation.factor.expression.ast");
    }
    let attached_ast = canonical_wire_factor_ast_bytes(ast)?;
    if !constant_time_eq(&attached_ast, &expression.canonical_json) {
        return binding_mismatch("specification.input.factor_evaluation.factor.expression.ast");
    }
    let computed_expression = domain_digest(FACTOR_AST_DOMAIN, &expression.canonical_json);
    if !constant_time_eq(
        factor_expression_id.as_bytes(),
        encode_digest(&computed_expression).as_bytes(),
    ) {
        return binding_mismatch(
            "specification.input.factor_evaluation.factor.expression.canonical_json",
        );
    }

    let registry = require_digest(
        factor.operator_registry_sha256.as_ref(),
        "specification.input.factor_evaluation.factor.operator_registry_sha256",
    )?;
    let direction = FactorDirection::try_from(factor.direction).map_err(|_| {
        JobValidationError::new(
            JobValidationCode::UnknownEnum,
            "specification.input.factor_evaluation.factor.direction",
        )
    })?;
    let direction = match direction {
        FactorDirection::HigherIsBetter => "higher_is_better",
        FactorDirection::LowerIsBetter => "lower_is_better",
        FactorDirection::Unspecified => {
            return invalid_input("specification.input.factor_evaluation.factor.direction");
        }
    };
    let policies = factor.frozen_policy.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.input.factor_evaluation.factor.frozen_policy",
        )
    })?;

    let mut output = String::with_capacity(2_048);
    output.push_str("{\"schema\":\"loop.factor-spec/v1\",\"expression_id\":\"");
    output.push_str(factor_expression_id);
    output.push_str("\",\"operator_registry_sha256\":\"");
    output.push_str(&encode_digest(&registry));
    output.push_str("\",\"direction\":\"");
    output.push_str(direction);
    output.push('"');
    write_factor_policy(
        &mut output,
        "universe_policy",
        policies.universe_policy.as_ref(),
    )?;
    write_factor_policy(&mut output, "data_policy", policies.data_policy.as_ref())?;
    write_factor_policy(
        &mut output,
        "calendar_policy",
        policies.calendar_policy.as_ref(),
    )?;
    write_factor_policy(
        &mut output,
        "preprocess_policy",
        policies.preprocess_policy.as_ref(),
    )?;
    write_factor_policy(
        &mut output,
        "neutralization_policy",
        policies.neutralization_policy.as_ref(),
    )?;
    write_factor_policy(
        &mut output,
        "portfolio_policy",
        policies.portfolio_policy.as_ref(),
    )?;
    write_factor_policy(
        &mut output,
        "execution_policy",
        policies.execution_policy.as_ref(),
    )?;
    write_factor_policy(&mut output, "cost_policy", policies.cost_policy.as_ref())?;
    write_factor_policy(
        &mut output,
        "evaluation_policy",
        policies.evaluation_policy.as_ref(),
    )?;
    output.push('}');
    Ok(output.into_bytes())
}

fn canonical_wire_factor_ast_bytes(ast: &FactorAst) -> Result<Vec<u8>, JobValidationError> {
    if ast.schema_version != 1 {
        return invalid_input("specification.input.factor_evaluation.factor.expression.ast");
    }
    let root = ast.root.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.input.factor_evaluation.factor.expression.ast.root",
        )
    })?;
    let mut writer = WireFactorAstWriter::default();
    writer.write_node(root, 1)?;
    if writer.output.len() > MAX_PROTOCOL_CANONICAL_AST_BYTES as usize {
        return invalid_input("specification.input.factor_evaluation.factor.expression.ast");
    }
    Ok(writer.output)
}

#[derive(Default)]
struct WireFactorAstWriter {
    output: Vec<u8>,
    nodes: u32,
}

impl WireFactorAstWriter {
    fn write_node(&mut self, node: &FactorAstNode, depth: u32) -> Result<(), JobValidationError> {
        if depth > MAX_PROTOCOL_AST_DEPTH {
            return invalid_input("specification.input.factor_evaluation.factor.expression.ast");
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_PROTOCOL_AST_NODES {
            return invalid_input("specification.input.factor_evaluation.factor.expression.ast");
        }

        match node.node.as_ref().ok_or_else(|| {
            JobValidationError::new(
                JobValidationCode::MissingField,
                "specification.input.factor_evaluation.factor.expression.ast.node",
            )
        })? {
            factor_ast_node::Node::Field(field) => {
                validate_wire_factor_identifier(&field.field)?;
                self.output
                    .extend_from_slice(b"{\"node\":\"field\",\"field\":\"");
                self.output.extend_from_slice(field.field.as_bytes());
                self.output.extend_from_slice(b"\"}");
            }
            factor_ast_node::Node::Literal(literal) => {
                match literal.value.as_ref().ok_or_else(|| {
                    JobValidationError::new(
                        JobValidationCode::MissingField,
                        "specification.input.factor_evaluation.factor.expression.ast.literal",
                    )
                })? {
                    factor_literal::Value::Decimal(decimal) => {
                        if CanonicalDecimal::new(decimal.value.clone()).is_err() {
                            return invalid_input(
                                "specification.input.factor_evaluation.factor.expression.ast.literal.decimal",
                            );
                        }
                        self.output
                            .extend_from_slice(b"{\"node\":\"decimal\",\"value\":\"");
                        self.output.extend_from_slice(decimal.value.as_bytes());
                        self.output.extend_from_slice(b"\"}");
                    }
                    factor_literal::Value::Boolean(value) => {
                        self.output.extend_from_slice(if *value {
                            b"{\"node\":\"boolean\",\"value\":true}"
                        } else {
                            b"{\"node\":\"boolean\",\"value\":false}"
                        });
                    }
                    factor_literal::Value::Enumeration(value) => {
                        validate_wire_factor_identifier(&value.enum_type)?;
                        validate_wire_factor_identifier(&value.value)?;
                        self.output
                            .extend_from_slice(b"{\"node\":\"enum\",\"enum_type\":\"");
                        self.output.extend_from_slice(value.enum_type.as_bytes());
                        self.output.extend_from_slice(b"\",\"value\":\"");
                        self.output.extend_from_slice(value.value.as_bytes());
                        self.output.extend_from_slice(b"\"}");
                    }
                }
            }
            factor_ast_node::Node::Call(call) => {
                let operator = call.operator.as_ref().ok_or_else(|| {
                    JobValidationError::new(
                        JobValidationCode::MissingField,
                        "specification.input.factor_evaluation.factor.expression.ast.call.operator",
                    )
                })?;
                validate_wire_factor_identifier(&operator.operator)?;
                PositiveInteger::new(operator.operator_version.clone()).map_err(|_| {
                    JobValidationError::new(
                        JobValidationCode::InvalidInput,
                        "specification.input.factor_evaluation.factor.expression.ast.call.operator_version",
                    )
                })?;
                if call.arguments.len() > MAX_PROTOCOL_AST_DIRECT_ARGUMENTS {
                    return invalid_input(
                        "specification.input.factor_evaluation.factor.expression.ast.call.arguments",
                    );
                }
                self.output
                    .extend_from_slice(b"{\"node\":\"call\",\"operator\":\"");
                self.output.extend_from_slice(operator.operator.as_bytes());
                self.output.extend_from_slice(b"\",\"operator_version\":\"");
                self.output
                    .extend_from_slice(operator.operator_version.as_bytes());
                self.output.extend_from_slice(b"\",\"arguments\":[");
                for (index, argument) in call.arguments.iter().enumerate() {
                    if index > 0 {
                        self.output.push(b',');
                    }
                    self.write_node(argument, depth + 1)?;
                }
                self.output.extend_from_slice(b"]}");
            }
        }
        Ok(())
    }
}

fn validate_wire_factor_identifier(value: &str) -> Result<(), JobValidationError> {
    Identifier::new(value.to_owned()).map(|_| ()).map_err(|_| {
        JobValidationError::new(
            JobValidationCode::InvalidInput,
            "specification.input.factor_evaluation.factor.expression.ast",
        )
    })
}

/// Compute the domain-separated digest claimed by an inline `factor_spec_id`.
pub fn factor_spec_identity_sha256(factor: &FactorSpec) -> Result<[u8; 32], JobValidationError> {
    canonical_factor_spec_identity_bytes(factor)
        .map(|canonical| domain_digest(FACTOR_SPEC_DOMAIN, &canonical))
}

/// Validate the two content-addressed identities carried by an inline factor.
pub fn validate_factor_spec_identity_envelope(
    factor: &FactorSpec,
) -> Result<(), JobValidationError> {
    let claimed = require_sha256_id(
        factor
            .factor_spec_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.input.factor_evaluation.factor.factor_spec_id",
    )?;
    let computed = factor_spec_identity_sha256(factor)?;
    if !constant_time_eq(claimed.as_bytes(), encode_digest(&computed).as_bytes()) {
        return binding_mismatch("specification.input.factor_evaluation.factor.factor_spec_id");
    }
    Ok(())
}

fn write_factor_policy(
    output: &mut String,
    name: &'static str,
    policy: Option<&PolicyReference>,
) -> Result<(), JobValidationError> {
    validate_policy_reference(
        policy,
        "specification.input.factor_evaluation.factor.frozen_policy",
    )?;
    let policy = policy.expect("validated policy reference");
    let policy_id = &policy
        .policy_id
        .as_ref()
        .expect("validated policy ID")
        .value;
    let digest = require_digest(
        policy.sha256.as_ref(),
        "specification.input.factor_evaluation.factor.frozen_policy",
    )?;
    output.push_str(",\"");
    output.push_str(name);
    output.push_str("\":{\"policy_id\":\"");
    output.push_str(policy_id);
    output.push_str("\",\"revision\":\"");
    output.push_str(&policy.revision);
    output.push_str("\",\"sha256\":\"");
    output.push_str(&encode_digest(&digest));
    output.push_str("\"}");
    Ok(())
}

fn validate_model_resolution(
    model: Option<&ModelResolutionSnapshot>,
    field: &'static str,
    submitted_at: (i64, i32),
) -> Result<(), JobValidationError> {
    let model =
        model.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    require_token_id(
        model
            .resolution_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        field,
    )?;
    require_token_id(
        model
            .provider_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        field,
    )?;
    require_token_id(
        model
            .model_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        field,
    )?;
    if !is_bounded_text(&model.requested_alias, MAX_PROTOCOL_NAME_BYTES)
        || !is_bounded_text(&model.provider_plugin_name, MAX_PROTOCOL_NAME_BYTES)
        || !is_build_version(&model.provider_plugin_version)
    {
        return invalid_input(field);
    }
    let family = ModelProtocolFamily::try_from(model.protocol_family)
        .map_err(|_| JobValidationError::new(JobValidationCode::UnknownEnum, field))?;
    if family == ModelProtocolFamily::Unspecified {
        return Err(JobValidationError::new(
            JobValidationCode::UnknownEnum,
            field,
        ));
    }
    let capabilities = model
        .capabilities
        .as_ref()
        .ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    if capabilities.context_window_tokens == 0
        || capabilities.context_window_tokens > MAX_JOB_TOKENS
        || capabilities.maximum_output_tokens == 0
        || capabilities.maximum_output_tokens > capabilities.context_window_tokens
    {
        return invalid_input(field);
    }
    let pricing = model
        .pricing
        .as_ref()
        .ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    validate_money(pricing.input_per_million_tokens.as_ref(), field)?;
    validate_money(pricing.output_per_million_tokens.as_ref(), field)?;
    validate_money(pricing.cached_input_per_million_tokens.as_ref(), field)?;
    require_digest(model.capability_sha256.as_ref(), field)?;
    require_digest(model.catalog_sha256.as_ref(), field)?;
    require_digest(model.provider_plugin_sha256.as_ref(), field)?;
    require_digest(model.snapshot_sha256.as_ref(), field)?;
    let resolved_at = require_timestamp(model.resolved_at.as_ref(), field)?;
    if resolved_at > submitted_at {
        return invalid_input(field);
    }
    Ok(())
}

fn validate_budget(
    budget: Option<&JobBudget>,
    field: &'static str,
) -> Result<(), JobValidationError> {
    let budget =
        budget.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    if budget.maximum_steps == 0
        || budget.maximum_steps > MAX_JOB_STEPS
        || budget.maximum_input_tokens > MAX_JOB_TOKENS
        || budget.maximum_output_tokens > MAX_JOB_TOKENS
    {
        return invalid_budget(field);
    }
    validate_money(budget.maximum_cost.as_ref(), field)?;
    let duration = budget
        .maximum_wall_time
        .as_ref()
        .ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    if duration.seconds < 0
        || duration.seconds > MAX_JOB_WALL_TIME_SECONDS
        || duration.nanos < 0
        || duration.nanos >= 1_000_000_000
        || (duration.seconds == 0 && duration.nanos == 0)
        || (duration.seconds == MAX_JOB_WALL_TIME_SECONDS && duration.nanos != 0)
    {
        return invalid_budget(field);
    }
    Ok(())
}

fn validate_money(
    money: Option<&crate::wire::v1::Money>,
    field: &'static str,
) -> Result<(), JobValidationError> {
    let money =
        money.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    if money.currency_code.len() != 3
        || !money
            .currency_code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
    {
        return invalid_budget(field);
    }
    let amount = money
        .amount
        .as_ref()
        .ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?
        .value
        .as_str();
    if !is_normalized_cost(amount) {
        return invalid_budget(field);
    }
    Ok(())
}

fn is_normalized_cost(amount: &str) -> bool {
    if amount.is_empty() || amount.starts_with(['-', '+']) {
        return false;
    }
    let mut parts = amount.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || (integer.len() > 1 && integer.starts_with('0'))
        || fraction.is_some_and(|value| {
            value.is_empty()
                || value.len() > 9
                || value.ends_with('0')
                || !value.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return false;
    }
    let significant = if integer == "0" {
        fraction.map_or(1, |value| value.trim_start_matches('0').len().max(1))
    } else {
        integer.len() + fraction.map_or(0, str::len)
    };
    significant <= 18
        && integer.parse::<u64>().is_ok_and(|value| value <= 1_000_000)
        && !(integer == "1000000" && fraction.is_some())
}

fn validate_provenance(
    provenance: Option<&ResearchProvenanceFingerprint>,
    field: &'static str,
) -> Result<(), JobValidationError> {
    let provenance = provenance
        .ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    for digest in [
        provenance.source_code_sha256.as_ref(),
        provenance.operator_registry_sha256.as_ref(),
        provenance.configuration_sha256.as_ref(),
        provenance.data_manifest_sha256.as_ref(),
        provenance.trading_calendar_sha256.as_ref(),
        provenance.environment_sha256.as_ref(),
    ] {
        if digest.is_none_or(|value| value.value.len() != 32) {
            return Err(JobValidationError::new(
                JobValidationCode::InvalidProvenance,
                field,
            ));
        }
    }
    Ok(())
}

fn validate_holdout_backtest_input(
    input: &HoldoutBacktestJobInput,
    submitted_at: (i64, i32),
) -> Result<(), JobValidationError> {
    let grant = input.consumed_grant.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.input.holdout_backtest.consumed_grant",
        )
    })?;
    let validity = validate_holdout_grant(grant)?;
    if submitted_at < validity.issued_at || submitted_at >= validity.expires_at {
        return invalid_input(
            "specification.input.holdout_backtest.consumed_grant.validity_window",
        );
    }
    if input.consumed_grant_revision == 0 {
        return invalid_input("specification.input.holdout_backtest.consumed_grant_revision");
    }
    let batch_id = require_token_id(
        input
            .job_batch_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        "specification.input.holdout_backtest.job_batch_id",
    )?;
    if batch_id
        == grant
            .holdout_grant_id
            .as_ref()
            .map_or("", |value| &value.value)
    {
        return binding_mismatch("specification.input.holdout_backtest.job_batch_id");
    }
    let plan_id = require_sha256_id(
        input
            .holdout_evaluation_plan_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        "specification.input.holdout_backtest.holdout_evaluation_plan_id",
    )?;
    let grant_plan_id = grant
        .holdout_evaluation_plan_id
        .as_ref()
        .map(|identity| identity.value.as_str())
        .ok_or_else(|| {
            JobValidationError::new(
                JobValidationCode::MissingField,
                "specification.input.holdout_backtest.consumed_grant.holdout_evaluation_plan_id",
            )
        })?;
    if plan_id != grant_plan_id {
        return binding_mismatch("specification.input.holdout_backtest.holdout_evaluation_plan_id");
    }
    let plan_digest = require_digest(
        input.evaluation_plan_sha256.as_ref(),
        "specification.input.holdout_backtest.evaluation_plan_sha256",
    )?;
    let grant_plan_digest = require_digest(
        grant.evaluation_plan_sha256.as_ref(),
        "specification.input.holdout_backtest.consumed_grant.evaluation_plan_sha256",
    )?;
    if plan_digest != grant_plan_digest {
        return binding_mismatch("specification.input.holdout_backtest.evaluation_plan_sha256");
    }
    if input.evaluation_plan_entry_index == 0
        || input.evaluation_plan_entry_index > grant.evaluation_plan_entry_count
    {
        return invalid_input("specification.input.holdout_backtest.evaluation_plan_entry_index");
    }
    let frozen = input.frozen_backtest_spec.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.input.holdout_backtest.frozen_backtest_spec",
        )
    })?;
    validate_frozen_backtest_spec(frozen, grant)?;
    validate_budget(
        input.budget.as_ref(),
        "specification.input.holdout_backtest.budget",
    )
}

/// Bind a locally validated holdout job to freshly verified canonical bytes.
///
/// This resolver-backed check is mandatory before persistence and before a
/// worker lease. It reparses both period and plan bytes on every call, so a
/// mutable or caller-constructed canonical DTO cannot cross this boundary. It
/// binds only the factor, budget, and `canonical_spec_sha256` claim to the plan
/// entry. The bytes, trusted schema digest, and artifact map must come from a
/// server-owned resolver, never from the request or worker. Before
/// materialization or dispatch, the Phase 7 owning BacktestSpec parser must use
/// the exact referenced artifact bytes to derive or validate the complete
/// `frozen_backtest_spec` and bind its sample, snapshots, return definition,
/// provenance, seed, and other fields. Persisted grant resolution and runtime
/// authorization remain external Phase 4 gates.
pub fn validate_holdout_backtest_plan_entry_binding(
    input: &HoldoutBacktestJobInput,
    submitted_at: &prost_types::Timestamp,
    canonical_period_bytes: &[u8],
    canonical_plan_bytes: &[u8],
    trusted_backtest_schema_sha256: &[u8; 32],
    resolved_backtest_artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<(), JobValidationError> {
    let submitted_at = require_timestamp(Some(submitted_at), "specification.submitted_at")?;
    validate_holdout_backtest_input(input, submitted_at)?;
    let grant = input
        .consumed_grant
        .as_ref()
        .expect("validated holdout grant");
    let plan_id = input
        .holdout_evaluation_plan_id
        .as_ref()
        .expect("validated plan ID")
        .value
        .as_str();
    let plan_digest = require_digest(
        input.evaluation_plan_sha256.as_ref(),
        "specification.input.holdout_backtest.evaluation_plan_sha256",
    )?;
    let grant_period_id = grant
        .holdout_period_id
        .as_ref()
        .expect("validated period ID")
        .value
        .as_str();
    let grant_period_digest = require_digest(
        grant.canonical_period_sha256.as_ref(),
        "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
    )?;
    let plan_binding_error = || {
        JobValidationError::new(
            JobValidationCode::BindingMismatch,
            "specification.input.holdout_backtest.resolved_plan",
        )
    };
    let period = verify_holdout_period_identity(
        canonical_period_bytes,
        grant_period_id,
        &grant_period_digest,
    )
    .map_err(|_| plan_binding_error())?;
    let plan = parse_canonical_holdout_evaluation_plan(
        canonical_plan_bytes,
        &period,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )
    .map_err(|_| plan_binding_error())?;
    if !constant_time_eq(
        plan_id.as_bytes(),
        plan.holdout_evaluation_plan_id.as_bytes(),
    ) || !constant_time_eq(&plan_digest, &plan.plan_sha256)
        || !constant_time_eq(
            grant_period_id.as_bytes(),
            plan.value.holdout_period_id.as_bytes(),
        )
        || !constant_time_eq(
            encode_digest(&grant_period_digest).as_bytes(),
            plan.value.canonical_period_sha256.as_bytes(),
        )
        || usize::try_from(grant.evaluation_plan_entry_count).ok() != Some(plan.value.entries.len())
    {
        return binding_mismatch("specification.input.holdout_backtest.resolved_plan");
    }
    let entry_index = input.evaluation_plan_entry_index.to_string();
    let entry = plan
        .value
        .entries
        .iter()
        .find(|entry| entry.entry_index == entry_index)
        .ok_or_else(|| {
            JobValidationError::new(
                JobValidationCode::BindingMismatch,
                "specification.input.holdout_backtest.evaluation_plan_entry_index",
            )
        })?;
    let frozen = input
        .frozen_backtest_spec
        .as_ref()
        .expect("validated frozen backtest spec");
    let factor_id = frozen
        .factor_spec_id
        .as_ref()
        .expect("validated factor ID")
        .value
        .as_str();
    let canonical_spec = require_digest(
        frozen.canonical_spec_sha256.as_ref(),
        "specification.input.holdout_backtest.frozen_backtest_spec.canonical_spec_sha256",
    )?;
    if !constant_time_eq(factor_id.as_bytes(), entry.factor_spec_id.as_bytes())
        || !constant_time_eq(
            encode_digest(&canonical_spec).as_bytes(),
            entry.backtest_spec_artifact.sha256.as_bytes(),
        )
        || !holdout_budget_matches_plan(
            input.budget.as_ref().expect("validated holdout budget"),
            &entry.job_budget,
        )
    {
        return binding_mismatch("specification.input.holdout_backtest.resolved_plan_entry");
    }
    Ok(())
}

fn holdout_budget_matches_plan(
    budget: &JobBudget,
    expected: &loop_core::holdout::HoldoutJobBudget,
) -> bool {
    let Some(cost) = budget.maximum_cost.as_ref() else {
        return false;
    };
    let Some(amount) = cost.amount.as_ref() else {
        return false;
    };
    let Some(wall_time) = budget.maximum_wall_time.as_ref() else {
        return false;
    };
    let wall_time_ns = i128::from(wall_time.seconds) * 1_000_000_000 + i128::from(wall_time.nanos);
    budget.maximum_steps.to_string() == expected.maximum_steps
        && budget.maximum_input_tokens.to_string() == expected.maximum_input_tokens
        && budget.maximum_output_tokens.to_string() == expected.maximum_output_tokens
        && amount.value == expected.maximum_cost.amount
        && cost.currency_code == expected.maximum_cost.currency_code
        && wall_time_ns.to_string() == expected.maximum_wall_time_ns
}

fn validate_holdout_grant(
    grant: &HoldoutGrantReference,
) -> Result<GrantValidityWindow, JobValidationError> {
    require_token_id(
        grant
            .holdout_grant_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        "specification.input.holdout_backtest.consumed_grant.holdout_grant_id",
    )?;
    let period_id = require_sha256_id(
        grant
            .holdout_period_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        "specification.input.holdout_backtest.consumed_grant.holdout_period_id",
    )?;
    require_digest(
        grant.freeze_manifest_sha256.as_ref(),
        "specification.input.holdout_backtest.consumed_grant.freeze_manifest_sha256",
    )?;
    let issued_at = require_timestamp(
        grant.issued_at.as_ref(),
        "specification.input.holdout_backtest.consumed_grant.issued_at",
    )?;
    let expires_at = require_timestamp(
        grant.expires_at.as_ref(),
        "specification.input.holdout_backtest.consumed_grant.expires_at",
    )?;
    if issued_at >= expires_at {
        return invalid_input("specification.input.holdout_backtest.consumed_grant.expires_at");
    }
    require_sha256_id(
        grant
            .holdout_evaluation_plan_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        "specification.input.holdout_backtest.consumed_grant.holdout_evaluation_plan_id",
    )?;
    require_digest(
        grant.evaluation_plan_sha256.as_ref(),
        "specification.input.holdout_backtest.consumed_grant.evaluation_plan_sha256",
    )?;
    if !(1..=MAX_HOLDOUT_ENTRIES).contains(&grant.evaluation_plan_entry_count) {
        return invalid_input(
            "specification.input.holdout_backtest.consumed_grant.evaluation_plan_entry_count",
        );
    }
    let period_digest = require_digest(
        grant.canonical_period_sha256.as_ref(),
        "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
    )?;
    if period_id != encode_digest(&period_digest) {
        return binding_mismatch(
            "specification.input.holdout_backtest.consumed_grant.canonical_period_sha256",
        );
    }
    Ok(GrantValidityWindow {
        issued_at,
        expires_at,
    })
}

fn validate_frozen_backtest_spec(
    backtest: &BacktestSpec,
    grant: &HoldoutGrantReference,
) -> Result<(), JobValidationError> {
    require_token_id(
        backtest
            .backtest_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        "specification.input.holdout_backtest.frozen_backtest_spec.backtest_id",
    )?;
    if backtest.schema_version != 1 {
        return invalid_input(
            "specification.input.holdout_backtest.frozen_backtest_spec.schema_version",
        );
    }
    require_sha256_id(
        backtest
            .factor_spec_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        "specification.input.holdout_backtest.frozen_backtest_spec.factor_spec_id",
    )?;
    if backtest.snapshot_ids.is_empty() || backtest.snapshot_ids.len() > MAX_DATASET_SNAPSHOTS {
        return invalid_input(
            "specification.input.holdout_backtest.frozen_backtest_spec.snapshot_ids",
        );
    }
    let mut previous: Option<&str> = None;
    for snapshot in &backtest.snapshot_ids {
        let value = require_sha256_id(
            Some(snapshot.value.as_str()),
            "specification.input.holdout_backtest.frozen_backtest_spec.snapshot_ids",
        )?;
        if previous.is_some_and(|prior| prior >= value) {
            return invalid_input(
                "specification.input.holdout_backtest.frozen_backtest_spec.snapshot_ids",
            );
        }
        previous = Some(value);
    }
    validate_locked_sample(backtest.sample.as_ref())?;
    validate_simple_return(
        backtest.return_definition,
        "specification.input.holdout_backtest.frozen_backtest_spec.return_definition",
    )?;
    validate_provenance(
        backtest.provenance.as_ref(),
        "specification.input.holdout_backtest.frozen_backtest_spec.provenance",
    )?;
    require_digest(
        backtest.canonical_spec_sha256.as_ref(),
        "specification.input.holdout_backtest.frozen_backtest_spec.canonical_spec_sha256",
    )?;
    require_digest(
        backtest.deterministic_seed.as_ref(),
        "specification.input.holdout_backtest.frozen_backtest_spec.deterministic_seed",
    )?;
    let created_at = require_timestamp(
        backtest.created_at.as_ref(),
        "specification.input.holdout_backtest.frozen_backtest_spec.created_at",
    )?;
    let grant_issued_at = require_timestamp(
        grant.issued_at.as_ref(),
        "specification.input.holdout_backtest.consumed_grant.issued_at",
    )?;
    if created_at > grant_issued_at {
        return binding_mismatch(
            "specification.input.holdout_backtest.frozen_backtest_spec.created_at",
        );
    }
    Ok(())
}

fn validate_locked_sample(
    sample: Option<&crate::wire::v1::SampleWindow>,
) -> Result<(), JobValidationError> {
    let sample = sample.ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "specification.input.holdout_backtest.frozen_backtest_spec.sample",
        )
    })?;
    let role = SampleRole::try_from(sample.role).map_err(|_| {
        JobValidationError::new(
            JobValidationCode::UnknownEnum,
            "specification.input.holdout_backtest.frozen_backtest_spec.sample.role",
        )
    })?;
    if !matches!(
        role,
        SampleRole::FirstLockedConfirmation | SampleRole::SecondLockedHistoricalHoldout
    ) {
        return invalid_input(
            "specification.input.holdout_backtest.frozen_backtest_spec.sample.role",
        );
    }
    let start = validate_civil_date(
        sample.start_inclusive.as_ref(),
        "specification.input.holdout_backtest.frozen_backtest_spec.sample.start_inclusive",
    )?;
    let end = validate_civil_date(
        sample.end_inclusive.as_ref(),
        "specification.input.holdout_backtest.frozen_backtest_spec.sample.end_inclusive",
    )?;
    if start > end {
        return invalid_input("specification.input.holdout_backtest.frozen_backtest_spec.sample");
    }
    Ok(())
}

fn validate_civil_date(
    date: Option<&crate::wire::v1::CivilDate>,
    field: &'static str,
) -> Result<(i32, u32, u32), JobValidationError> {
    let date =
        date.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    let leap = date.year % 4 == 0 && (date.year % 100 != 0 || date.year % 400 == 0);
    let maximum_day = match date.month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return invalid_input(field),
    };
    if !(1..=9_999).contains(&date.year) || !(1..=maximum_day).contains(&date.day) {
        return invalid_input(field);
    }
    Ok((date.year, date.month, date.day))
}

fn validate_simple_return(value: i32, field: &'static str) -> Result<(), JobValidationError> {
    let definition = ReturnDefinition::try_from(value)
        .map_err(|_| JobValidationError::new(JobValidationCode::UnknownEnum, field))?;
    if definition != ReturnDefinition::SimpleNavReturn {
        return invalid_input(field);
    }
    Ok(())
}

fn require_digest(
    digest: Option<&Sha256Digest>,
    field: &'static str,
) -> Result<[u8; 32], JobValidationError> {
    let bytes = digest
        .ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?
        .value
        .as_slice();
    bytes
        .try_into()
        .map_err(|_| JobValidationError::new(JobValidationCode::InvalidIdentity, field))
}

fn encode_digest(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn is_protocol_package(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_PROTOCOL_NAME_BYTES || !value.is_ascii() {
        return false;
    }
    let mut parts = value.split('.').peekable();
    let mut count = 0;
    while let Some(part) = parts.next() {
        count += 1;
        let is_version = parts.peek().is_none();
        if is_version {
            if !part.strip_prefix('v').is_some_and(|digits| {
                digits
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_digit() && byte != b'0')
                    && digits.bytes().all(|byte| byte.is_ascii_digit())
            }) {
                return false;
            }
        } else if !is_lower_identifier_segment(part) {
            return false;
        }
    }
    count >= 2
}

fn is_protocol_feature(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROTOCOL_NAME_BYTES
        && value.is_ascii()
        && value.split('.').count() >= 2
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && segment
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && segment
                    .bytes()
                    .last()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn is_build_version(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_BUILD_VERSION_BYTES || !value.is_ascii() {
        return false;
    }
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'_' | b'-'))
}

fn is_lower_identifier_segment(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_policy_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= MAX_ID_BYTES
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
        })
}

fn invalid_protocol_selection<T>(field: &'static str) -> Result<T, JobValidationError> {
    Err(JobValidationError::new(
        JobValidationCode::InvalidProtocolSelection,
        field,
    ))
}

fn invalid_budget<T>(field: &'static str) -> Result<T, JobValidationError> {
    Err(JobValidationError::new(
        JobValidationCode::InvalidBudget,
        field,
    ))
}

fn invalid_input<T>(field: &'static str) -> Result<T, JobValidationError> {
    Err(JobValidationError::new(
        JobValidationCode::InvalidInput,
        field,
    ))
}

fn binding_mismatch<T>(field: &'static str) -> Result<T, JobValidationError> {
    Err(JobValidationError::new(
        JobValidationCode::BindingMismatch,
        field,
    ))
}

/// Validate a complete record's cross-field and minimum terminal invariants.
///
/// Canonical factor verification remains the factor domain's responsibility.
/// This boundary requires a strict factor ID and proves that a rejection is
/// bound to the exact factor reference carried by an eligible job input.
pub fn validate_job_record(record: &JobRecord) -> Result<ValidatedJobShape, JobValidationError> {
    let specification = record
        .specification
        .as_ref()
        .ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, "specification"))?;
    if record.revision == 0 {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidRevision,
            "revision",
        ));
    }
    let kind = validate_job_specification(specification)?.kind;
    let specification_job_id = require_token_id(
        specification
            .job_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "specification.job_id",
    )?;
    let submitted_at = require_timestamp(
        specification.submitted_at.as_ref(),
        "specification.submitted_at",
    )?;
    let updated_at = require_timestamp(record.updated_at.as_ref(), "updated_at")?;
    if updated_at < submitted_at {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidEnvelope,
            "updated_at",
        ));
    }

    let state = JobState::try_from(record.state)
        .map_err(|_| JobValidationError::new(JobValidationCode::UnknownEnum, "state"))?;
    if state == JobState::Unspecified {
        return Err(JobValidationError::new(
            JobValidationCode::UnknownEnum,
            "state",
        ));
    }

    let has_lease = record.active_lease.is_some();
    let is_active = matches!(state, JobState::Leased | JobState::Running);
    let is_terminal = is_terminal_state(state);
    if has_lease != is_active {
        return Err(JobValidationError::new(
            JobValidationCode::StateLeaseMismatch,
            "active_lease",
        ));
    }
    if state == JobState::Queued {
        if record.attempt != 0 {
            return Err(JobValidationError::new(
                JobValidationCode::InvalidAttempt,
                "attempt",
            ));
        }
    } else if (is_active || is_terminal)
        && !matches!(state, JobState::Cancelled | JobState::BudgetExhausted)
        && record.attempt == 0
    {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidAttempt,
            "attempt",
        ));
    }

    let outcome = match record.outcome.as_ref() {
        Some(wrapper) => Some(wrapper.outcome.as_ref().ok_or_else(|| {
            JobValidationError::new(JobValidationCode::MissingField, "outcome.outcome")
        })?),
        None => None,
    };
    validate_state_outcome(state, outcome)?;

    if let Some(outcome) = outcome {
        validate_terminal_payload(record, specification, kind, outcome)?;
    }
    if let Some(lease) = record.active_lease.as_ref() {
        validate_lease(
            lease,
            specification_job_id,
            record.revision,
            submitted_at,
            updated_at,
        )?;
    }

    Ok(ValidatedJobShape { kind, state })
}

fn is_terminal_state(state: JobState) -> bool {
    matches!(
        state,
        JobState::Succeeded
            | JobState::FactorRejected
            | JobState::InfrastructureFailed
            | JobState::Cancelled
            | JobState::BudgetExhausted
    )
}

fn validate_state_outcome(
    state: JobState,
    outcome: Option<&job_outcome::Outcome>,
) -> Result<(), JobValidationError> {
    if matches!(
        state,
        JobState::Queued | JobState::Leased | JobState::Running
    ) {
        if outcome.is_some() {
            return Err(JobValidationError::new(
                JobValidationCode::StateOutcomeMismatch,
                "outcome",
            ));
        }
        return Ok(());
    }
    let outcome = outcome.ok_or_else(|| {
        JobValidationError::new(JobValidationCode::StateOutcomeMismatch, "outcome")
    })?;
    let expected = matches!(
        (state, outcome),
        (JobState::Succeeded, job_outcome::Outcome::Success(_))
            | (
                JobState::FactorRejected,
                job_outcome::Outcome::FactorRejection(_)
            )
            | (
                JobState::InfrastructureFailed,
                job_outcome::Outcome::InfrastructureFailure(_)
            )
            | (JobState::Cancelled, job_outcome::Outcome::Cancellation(_))
            | (
                JobState::BudgetExhausted,
                job_outcome::Outcome::BudgetExhaustion(_)
            )
    );
    if expected {
        Ok(())
    } else {
        Err(JobValidationError::new(
            JobValidationCode::StateOutcomeMismatch,
            "outcome",
        ))
    }
}

fn validate_terminal_payload(
    record: &JobRecord,
    specification: &JobSpecification,
    kind: JobKind,
    outcome: &job_outcome::Outcome,
) -> Result<(), JobValidationError> {
    let submitted_at = require_timestamp(
        specification.submitted_at.as_ref(),
        "specification.submitted_at",
    )?;
    let updated_at = require_timestamp(record.updated_at.as_ref(), "updated_at")?;
    match outcome {
        job_outcome::Outcome::Success(success) => {
            validate_artifacts(&success.outputs, "outcome.success.outputs")
        }
        job_outcome::Outcome::FactorRejection(rejection) => {
            validate_factor_rejection(specification, kind, rejection)?;
            let rejected_at = require_timestamp(
                rejection.rejected_at.as_ref(),
                "outcome.factor_rejection.rejected_at",
            )?;
            validate_event_timestamp(
                rejected_at,
                submitted_at,
                updated_at,
                "outcome.factor_rejection.rejected_at",
            )
        }
        job_outcome::Outcome::InfrastructureFailure(failure) => {
            validate_infrastructure_failure(record.attempt, failure)?;
            let failed_at = require_timestamp(
                failure.failed_at.as_ref(),
                "outcome.infrastructure_failure.failed_at",
            )?;
            validate_event_timestamp(
                failed_at,
                submitted_at,
                updated_at,
                "outcome.infrastructure_failure.failed_at",
            )
        }
        job_outcome::Outcome::Cancellation(cancellation) => {
            validate_cancellation(cancellation)?;
            let cancelled_at = require_timestamp(
                cancellation.cancelled_at.as_ref(),
                "outcome.cancellation.cancelled_at",
            )?;
            validate_event_timestamp(
                cancelled_at,
                submitted_at,
                updated_at,
                "outcome.cancellation.cancelled_at",
            )
        }
        job_outcome::Outcome::BudgetExhaustion(exhaustion) => {
            if !is_bounded_text(&exhaustion.exhausted_limit, MAX_ERROR_CODE_BYTES) {
                return Err(JobValidationError::new(
                    JobValidationCode::InvalidTerminalPayload,
                    "outcome.budget_exhaustion.exhausted_limit",
                ));
            }
            validate_budget(
                exhaustion.enforced_budget.as_ref(),
                "outcome.budget_exhaustion.enforced_budget",
            )?;
            if exhaustion.enforced_budget.as_ref() != Some(job_budget(specification)) {
                return binding_mismatch("outcome.budget_exhaustion.enforced_budget");
            }
            let exhausted_at = require_timestamp(
                exhaustion.exhausted_at.as_ref(),
                "outcome.budget_exhaustion.exhausted_at",
            )?;
            validate_event_timestamp(
                exhausted_at,
                submitted_at,
                updated_at,
                "outcome.budget_exhaustion.exhausted_at",
            )
        }
    }
}

fn job_budget(specification: &JobSpecification) -> &JobBudget {
    match specification.input.as_ref().expect("validated job input") {
        job_specification::Input::Discovery(value) => value.budget.as_ref(),
        job_specification::Input::FactorEvaluation(value) => value.budget.as_ref(),
        job_specification::Input::Backtest(value) => value.budget.as_ref(),
        job_specification::Input::Reconciliation(value) => value.budget.as_ref(),
        job_specification::Input::HoldoutBacktest(value) => value.budget.as_ref(),
        job_specification::Input::Artifact(value) => value.budget.as_ref(),
    }
    .expect("validated job budget")
}

fn validate_event_timestamp(
    event_at: (i64, i32),
    submitted_at: (i64, i32),
    updated_at: (i64, i32),
    field: &'static str,
) -> Result<(), JobValidationError> {
    if event_at < submitted_at || event_at > updated_at {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidEnvelope,
            field,
        ));
    }
    Ok(())
}

fn validate_factor_rejection(
    specification: &JobSpecification,
    kind: JobKind,
    rejection: &FactorRejection,
) -> Result<(), JobValidationError> {
    if !matches!(
        kind,
        JobKind::FactorEvaluation | JobKind::Backtest | JobKind::HoldoutBacktest
    ) {
        return Err(JobValidationError::new(
            JobValidationCode::RejectionNotAllowed,
            "outcome.factor_rejection",
        ));
    }
    let expected = factor_id_from_input(specification)?;
    let actual = require_sha256_id(
        rejection
            .factor_spec_id
            .as_ref()
            .map(|value| value.value.as_str()),
        "outcome.factor_rejection.factor_spec_id",
    )?;
    if !constant_time_eq(expected.as_bytes(), actual.as_bytes()) {
        return Err(JobValidationError::new(
            JobValidationCode::FactorIdentityMismatch,
            "outcome.factor_rejection.factor_spec_id",
        ));
    }
    let code = FactorRejectionCode::try_from(rejection.code).map_err(|_| {
        JobValidationError::new(
            JobValidationCode::UnknownEnum,
            "outcome.factor_rejection.code",
        )
    })?;
    if code == FactorRejectionCode::Unspecified {
        return Err(JobValidationError::new(
            JobValidationCode::UnknownEnum,
            "outcome.factor_rejection.code",
        ));
    }
    if !is_bounded_text(&rejection.reason, MAX_REASON_BYTES) {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidTerminalPayload,
            "outcome.factor_rejection.reason",
        ));
    }
    require_timestamp(
        rejection.rejected_at.as_ref(),
        "outcome.factor_rejection.rejected_at",
    )?;
    validate_artifacts(&rejection.evidence, "outcome.factor_rejection.evidence")
}

fn factor_id_from_input(specification: &JobSpecification) -> Result<&str, JobValidationError> {
    let value = match specification.input.as_ref() {
        Some(job_specification::Input::FactorEvaluation(input)) => input
            .factor
            .as_ref()
            .and_then(|factor| factor.factor_spec_id.as_ref())
            .map(|identity| identity.value.as_str()),
        Some(job_specification::Input::Backtest(input)) => input
            .factor_spec_id
            .as_ref()
            .map(|identity| identity.value.as_str()),
        Some(job_specification::Input::HoldoutBacktest(input)) => input
            .frozen_backtest_spec
            .as_ref()
            .and_then(|backtest| backtest.factor_spec_id.as_ref())
            .map(|identity| identity.value.as_str()),
        _ => None,
    };
    require_sha256_id(value, "specification.input.factor_spec_id")
}

fn validate_infrastructure_failure(
    record_attempt: u32,
    failure: &InfrastructureFailure,
) -> Result<(), JobValidationError> {
    let error = failure.error.as_ref().ok_or_else(|| {
        JobValidationError::new(
            JobValidationCode::MissingField,
            "outcome.infrastructure_failure.error",
        )
    })?;
    validate_service_error(error)?;
    if failure.attempt == 0 || failure.attempt != record_attempt {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidTerminalPayload,
            "outcome.infrastructure_failure.attempt",
        ));
    }
    require_timestamp(
        failure.failed_at.as_ref(),
        "outcome.infrastructure_failure.failed_at",
    )?;
    Ok(())
}

pub(crate) fn validate_service_error(error: &ServiceError) -> Result<(), JobValidationError> {
    let category = ErrorCategory::try_from(error.category).map_err(|_| {
        JobValidationError::new(
            JobValidationCode::UnknownEnum,
            "outcome.infrastructure_failure.error.category",
        )
    })?;
    if category == ErrorCategory::Unspecified {
        return Err(JobValidationError::new(
            JobValidationCode::UnknownEnum,
            "outcome.infrastructure_failure.error.category",
        ));
    }
    if !is_stable_error_code(&error.code)
        || !is_bounded_text(&error.message, MAX_ERROR_MESSAGE_BYTES)
    {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidTerminalPayload,
            "outcome.infrastructure_failure.error",
        ));
    }
    if error.details.len() > MAX_ERROR_DETAILS {
        return Err(JobValidationError::new(
            JobValidationCode::CollectionLimit,
            "outcome.infrastructure_failure.error.details",
        ));
    }
    for detail in &error.details {
        if !is_bounded_field_path(&detail.field_path)
            || !is_stable_error_code(&detail.code)
            || !is_bounded_text(&detail.message, MAX_ERROR_MESSAGE_BYTES)
        {
            return Err(JobValidationError::new(
                JobValidationCode::InvalidTerminalPayload,
                "outcome.infrastructure_failure.error.details",
            ));
        }
    }
    Ok(())
}

fn validate_cancellation(cancellation: &JobCancellation) -> Result<(), JobValidationError> {
    if !is_bounded_text(&cancellation.reason, MAX_REASON_BYTES) {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidTerminalPayload,
            "outcome.cancellation.reason",
        ));
    }
    validate_actor(
        cancellation.cancelled_by.as_ref(),
        "outcome.cancellation.cancelled_by",
    )?;
    require_timestamp(
        cancellation.cancelled_at.as_ref(),
        "outcome.cancellation.cancelled_at",
    )?;
    Ok(())
}

fn validate_lease(
    lease: &crate::wire::v1::JobLease,
    specification_job_id: &str,
    record_revision: u64,
    submitted_at: (i64, i32),
    record_updated_at: (i64, i32),
) -> Result<(), JobValidationError> {
    require_token_id(
        lease.lease_id.as_ref().map(|value| value.value.as_str()),
        "active_lease.lease_id",
    )?;
    let lease_job_id = require_token_id(
        lease.job_id.as_ref().map(|value| value.value.as_str()),
        "active_lease.job_id",
    )?;
    if lease_job_id != specification_job_id {
        return Err(JobValidationError::new(
            JobValidationCode::LeaseJobMismatch,
            "active_lease.job_id",
        ));
    }
    validate_actor(lease.owner.as_ref(), "active_lease.owner")?;
    if lease.acquired_revision == 0 || lease.acquired_revision > record_revision {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidRevision,
            "active_lease.acquired_revision",
        ));
    }
    let issued = require_timestamp(lease.issued_at.as_ref(), "active_lease.issued_at")?;
    let heartbeat = require_timestamp(lease.heartbeat_at.as_ref(), "active_lease.heartbeat_at")?;
    let expires = require_timestamp(lease.expires_at.as_ref(), "active_lease.expires_at")?;
    if issued < submitted_at
        || issued > heartbeat
        || heartbeat > record_updated_at
        || record_updated_at >= expires
    {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidLease,
            "active_lease.timestamps",
        ));
    }
    Ok(())
}

fn validate_actor(actor: Option<&Actor>, field: &'static str) -> Result<(), JobValidationError> {
    let actor =
        actor.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    require_token_id(
        actor.actor_id.as_ref().map(|value| value.value.as_str()),
        field,
    )?;
    let kind = ActorKind::try_from(actor.kind)
        .map_err(|_| JobValidationError::new(JobValidationCode::UnknownEnum, field))?;
    if kind == ActorKind::Unspecified {
        return Err(JobValidationError::new(
            JobValidationCode::UnknownEnum,
            field,
        ));
    }
    if (!actor.display_name.is_empty()
        && !is_bounded_text(&actor.display_name, MAX_ACTOR_DISPLAY_NAME_BYTES))
        || !is_bounded_text(
            &actor.authenticated_subject,
            MAX_AUTHENTICATED_SUBJECT_BYTES,
        )
    {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidEnvelope,
            field,
        ));
    }
    Ok(())
}

fn validate_artifacts(
    artifacts: &[crate::wire::v1::ArtifactRef],
    field: &'static str,
) -> Result<(), JobValidationError> {
    if artifacts.len() > MAX_OUTCOME_ARTIFACTS {
        return Err(JobValidationError::new(
            JobValidationCode::CollectionLimit,
            field,
        ));
    }
    if artifacts
        .iter()
        .any(|artifact| validate_artifact_ref(artifact).is_err())
    {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidTerminalPayload,
            field,
        ));
    }
    Ok(())
}

fn require_sha256_id<'a>(
    value: Option<&'a str>,
    field: &'static str,
) -> Result<&'a str, JobValidationError> {
    let value =
        value.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidIdentity,
            field,
        ));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidIdentity,
            field,
        ));
    }
    Ok(value)
}

fn require_token_id<'a>(
    value: Option<&'a str>,
    field: &'static str,
) -> Result<&'a str, JobValidationError> {
    let value =
        value.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidIdentity,
            field,
        ));
    };
    if value.len() > MAX_ID_BYTES
        || !first.is_ascii_alphanumeric()
        || !bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidIdentity,
            field,
        ));
    }
    Ok(value)
}

fn require_timestamp(
    timestamp: Option<&prost_types::Timestamp>,
    field: &'static str,
) -> Result<(i64, i32), JobValidationError> {
    let timestamp =
        timestamp.ok_or_else(|| JobValidationError::new(JobValidationCode::MissingField, field))?;
    if !(MIN_TIMESTAMP_SECONDS..=MAX_TIMESTAMP_SECONDS).contains(&timestamp.seconds)
        || !(0..1_000_000_000).contains(&timestamp.nanos)
    {
        return Err(JobValidationError::new(
            JobValidationCode::InvalidTerminalPayload,
            field,
        ));
    }
    Ok((timestamp.seconds, timestamp.nanos))
}

fn is_bounded_text(value: &str, maximum_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_bytes
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn is_stable_error_code(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_ERROR_CODE_BYTES {
        return false;
    }
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
        })
}

fn is_bounded_field_path(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_ERROR_FIELD_PATH_BYTES
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}
