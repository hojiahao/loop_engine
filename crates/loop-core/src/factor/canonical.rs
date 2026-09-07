use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    CanonicalizationError, EnumLiteral, ExpressionId, FactorDirection, FactorExpr, FactorSpec,
    FactorSpecDraft, FactorSpecId, IdentityVerificationError, Literal, OperatorCall,
    OperatorDefinition, OperatorPolicyRegistry, OperatorRegistryId, PolicyId, PolicyRef,
    PositiveInteger, ValidationError, ValidationLimits, ValueType, constant_time_digest_eq,
    encode_digest,
};

const EXPRESSION_DOMAIN: &[u8] = b"loop.factor-ast/v1\0";
const FACTOR_SPEC_DOMAIN: &[u8] = b"loop.factor-spec/v1\0";
const OPERATOR_REGISTRY_DOMAIN: &[u8] = b"loop.operator-registry/v1\0";

pub fn canonicalize_expression(
    expression: &FactorExpr,
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
) -> Result<FactorExpr, CanonicalizationError> {
    validate_submitted_expression(expression, registry, limits)?;
    let normalized = normalize_expression(expression, registry);
    validate_expression(&normalized, registry, limits)?;

    let mut bytes = Vec::new();
    write_expression(&mut bytes, &normalized);
    validate_canonical_size(bytes.len(), limits)?;
    parse_canonical_expression(&bytes, registry, limits)
}

pub fn canonical_expression_bytes(
    expression: &FactorExpr,
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
) -> Result<Vec<u8>, CanonicalizationError> {
    let normalized = canonicalize_expression(expression, registry, limits)?;
    let mut output = Vec::new();
    write_expression(&mut output, &normalized);
    validate_canonical_size(output.len(), limits)?;
    Ok(output)
}

/// Parses only the closed canonical v1 byte representation.
///
/// The parsed value is normalized and emitted again; any alternate field
/// order, whitespace, unknown/duplicate field, or non-canonical spelling is
/// rejected before the evaluator can receive it.
pub fn parse_canonical_expression(
    canonical: &[u8],
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
) -> Result<FactorExpr, CanonicalizationError> {
    validate_canonical_size(canonical.len(), limits)?;
    let raw: RawNode = serde_json::from_slice(canonical)
        .map_err(|error| CanonicalizationError::Parse(error.to_string()))?;
    let parsed = raw.try_into_domain()?;
    validate_submitted_expression(&parsed, registry, limits)?;
    let normalized = normalize_expression(&parsed, registry);
    validate_expression(&normalized, registry, limits)?;
    let mut emitted = Vec::new();
    write_expression(&mut emitted, &normalized);
    validate_canonical_size(emitted.len(), limits)?;
    if emitted != canonical {
        return Err(CanonicalizationError::NonCanonical);
    }
    Ok(normalized)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "node", deny_unknown_fields)]
enum RawNode {
    #[serde(rename = "field")]
    Field { field: String },
    #[serde(rename = "decimal")]
    Decimal { value: String },
    #[serde(rename = "boolean")]
    Boolean { value: bool },
    #[serde(rename = "enum")]
    Enumeration { enum_type: String, value: String },
    #[serde(rename = "call")]
    Call {
        operator: String,
        operator_version: String,
        arguments: Vec<RawNode>,
    },
}

impl RawNode {
    fn try_into_domain(self) -> Result<FactorExpr, CanonicalizationError> {
        match self {
            Self::Field { field } => Ok(FactorExpr::Field(super::FieldRef::new(
                super::Identifier::new(field)
                    .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
            ))),
            Self::Decimal { value } => Ok(FactorExpr::Literal(Literal::Decimal(
                super::CanonicalDecimal::new(value)
                    .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
            ))),
            Self::Boolean { value } => Ok(FactorExpr::Literal(Literal::Boolean(value))),
            Self::Enumeration { enum_type, value } => {
                Ok(FactorExpr::Literal(Literal::Enumeration(EnumLiteral::new(
                    super::Identifier::new(enum_type)
                        .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
                    super::Identifier::new(value)
                        .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
                ))))
            }
            Self::Call {
                operator,
                operator_version,
                arguments,
            } => Ok(FactorExpr::Call(OperatorCall::new(
                super::OperatorRef::new(
                    super::Identifier::new(operator)
                        .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
                    super::PositiveInteger::new(operator_version)
                        .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
                ),
                arguments
                    .into_iter()
                    .map(Self::try_into_domain)
                    .collect::<Result<Vec<_>, _>>()?,
            ))),
        }
    }
}

pub fn expression_id(
    expression: &FactorExpr,
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
) -> Result<ExpressionId, CanonicalizationError> {
    let canonical = canonical_expression_bytes(expression, registry, limits)?;
    Ok(ExpressionId::from_bytes(domain_hash(
        EXPRESSION_DOMAIN,
        &canonical,
    )))
}

pub fn canonical_operator_registry_bytes(registry: &OperatorPolicyRegistry) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(b"{\"schema\":\"loop.operator-registry/v1\",\"fields\":[");
    for (index, (field, value_type)) in registry.fields.iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        output.extend_from_slice(b"{\"field\":\"");
        output.extend_from_slice(field.as_str().as_bytes());
        output.extend_from_slice(b"\",\"outputType\":");
        write_registry_value_type(&mut output, value_type);
        output.push(b'}');
    }
    output.extend_from_slice(b"],\"enums\":[");
    for (index, (enum_type, values)) in registry.enums.iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        output.extend_from_slice(b"{\"enumType\":\"");
        output.extend_from_slice(enum_type.as_str().as_bytes());
        output.extend_from_slice(b"\",\"values\":[");
        for (value_index, value) in values.iter().enumerate() {
            if value_index > 0 {
                output.push(b',');
            }
            output.push(b'"');
            output.extend_from_slice(value.as_str().as_bytes());
            output.push(b'"');
        }
        output.extend_from_slice(b"]}");
    }
    output.extend_from_slice(b"],\"operators\":[");
    let mut operators: Vec<_> = registry.operators.values().collect();
    operators.sort_by(|left, right| {
        left.operator
            .name()
            .cmp(right.operator.name())
            .then_with(|| {
                compare_positive_integers(
                    left.operator.semantic_version(),
                    right.operator.semantic_version(),
                )
            })
    });
    for (index, definition) in operators.into_iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        write_registry_operator(&mut output, definition);
    }
    output.extend_from_slice(b"]}");
    output
}

pub fn operator_registry_id(registry: &OperatorPolicyRegistry) -> OperatorRegistryId {
    OperatorRegistryId::from_bytes(domain_hash(
        OPERATOR_REGISTRY_DOMAIN,
        &canonical_operator_registry_bytes(registry),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFactorSpec {
    schema: String,
    expression_id: String,
    operator_registry_sha256: String,
    direction: FactorDirection,
    universe_policy: RawPolicyRef,
    data_policy: RawPolicyRef,
    calendar_policy: RawPolicyRef,
    preprocess_policy: RawPolicyRef,
    neutralization_policy: RawPolicyRef,
    portfolio_policy: RawPolicyRef,
    execution_policy: RawPolicyRef,
    cost_policy: RawPolicyRef,
    evaluation_policy: RawPolicyRef,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicyRef {
    policy_id: String,
    revision: String,
    sha256: String,
}

impl RawFactorSpec {
    fn try_into_draft(self) -> Result<FactorSpecDraft, CanonicalizationError> {
        if self.schema != "loop.factor-spec/v1" {
            return Err(CanonicalizationError::Parse(
                "FactorSpec schema must be loop.factor-spec/v1".to_owned(),
            ));
        }
        Ok(FactorSpecDraft::new(
            ExpressionId::parse(&self.expression_id)
                .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
            parse_sha256(&self.operator_registry_sha256)?,
            self.direction,
            self.universe_policy.try_into_domain()?,
            self.data_policy.try_into_domain()?,
            self.calendar_policy.try_into_domain()?,
            self.preprocess_policy.try_into_domain()?,
            self.neutralization_policy.try_into_domain()?,
            self.portfolio_policy.try_into_domain()?,
            self.execution_policy.try_into_domain()?,
            self.cost_policy.try_into_domain()?,
            self.evaluation_policy.try_into_domain()?,
        ))
    }
}

impl RawPolicyRef {
    fn try_into_domain(self) -> Result<PolicyRef, CanonicalizationError> {
        Ok(PolicyRef::new(
            PolicyId::new(self.policy_id)
                .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
            PositiveInteger::new(self.revision)
                .map_err(|error| CanonicalizationError::Parse(error.to_string()))?,
            parse_sha256(&self.sha256)?,
        ))
    }
}

/// Bind an untrusted draft to the exact canonical expression it references.
///
/// Scalar and enum expressions remain valid stored ASTs, but they cannot form
/// a factor specification because the registry must resolve the root to
/// `series`.
pub fn bind_factor_spec(
    draft: FactorSpecDraft,
    canonical_expression: &[u8],
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
) -> Result<FactorSpec, IdentityVerificationError> {
    let expression = parse_canonical_expression(canonical_expression, registry, limits)?;
    if !constant_time_digest_eq(
        draft.operator_registry_sha256(),
        registry.identity().as_bytes(),
    ) {
        return Err(CanonicalizationError::OperatorRegistryMismatch {
            claimed: format!("sha256:{}", encode_digest(draft.operator_registry_sha256())),
            resolved: registry.identity().to_external(),
        }
        .into());
    }
    draft
        .expression_id()
        .verify(&expression, registry, limits)?;
    let root_type =
        validate_expression(&expression, registry, limits).map_err(CanonicalizationError::from)?;
    if root_type != ValueType::Series {
        return Err(CanonicalizationError::NonSeriesFactorRoot {
            actual: root_type.label(),
        }
        .into());
    }
    Ok(FactorSpec::from_verified(draft, expression))
}

/// Strictly parse canonical specification bytes and jointly verify the linked
/// canonical expression under the supplied registry.
pub fn parse_canonical_factor_spec(
    canonical_spec: &[u8],
    expected_factor_spec_id: FactorSpecId,
    canonical_expression: &[u8],
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
) -> Result<FactorSpec, IdentityVerificationError> {
    validate_canonical_size(canonical_spec.len(), limits).map_err(CanonicalizationError::from)?;
    let raw: RawFactorSpec = serde_json::from_slice(canonical_spec)
        .map_err(|error| CanonicalizationError::Parse(error.to_string()))?;
    let draft = raw.try_into_draft()?;
    let emitted = canonical_factor_spec_draft_bytes(&draft);
    if emitted != canonical_spec {
        return Err(CanonicalizationError::NonCanonical.into());
    }
    let bound = bind_factor_spec(draft, canonical_expression, registry, limits)?;
    let computed = factor_spec_id(&bound);
    if !constant_time_digest_eq(expected_factor_spec_id.as_bytes(), computed.as_bytes()) {
        return Err(IdentityVerificationError::FactorSpecMismatch {
            claimed: expected_factor_spec_id,
            computed,
        });
    }
    Ok(bound)
}

pub fn canonical_factor_spec_bytes(spec: &FactorSpec) -> Vec<u8> {
    canonical_factor_spec_draft_bytes(&spec.draft)
}

fn canonical_factor_spec_draft_bytes(spec: &FactorSpecDraft) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(b"{\"schema\":\"loop.factor-spec/v1\",\"expression_id\":\"");
    output.extend_from_slice(spec.expression_id().to_string().as_bytes());
    output.extend_from_slice(b"\",\"operator_registry_sha256\":\"sha256:");
    output.extend_from_slice(encode_digest(spec.operator_registry_sha256()).as_bytes());
    output.extend_from_slice(b"\",\"direction\":\"");
    output.extend_from_slice(match spec.direction() {
        FactorDirection::HigherIsBetter => b"higher_is_better",
        FactorDirection::LowerIsBetter => b"lower_is_better",
    });
    output.extend_from_slice(b"\",\"universe_policy\":");
    write_policy(&mut output, spec.universe_policy());
    output.extend_from_slice(b",\"data_policy\":");
    write_policy(&mut output, spec.data_policy());
    output.extend_from_slice(b",\"calendar_policy\":");
    write_policy(&mut output, spec.calendar_policy());
    output.extend_from_slice(b",\"preprocess_policy\":");
    write_policy(&mut output, spec.preprocess_policy());
    output.extend_from_slice(b",\"neutralization_policy\":");
    write_policy(&mut output, spec.neutralization_policy());
    output.extend_from_slice(b",\"portfolio_policy\":");
    write_policy(&mut output, spec.portfolio_policy());
    output.extend_from_slice(b",\"execution_policy\":");
    write_policy(&mut output, spec.execution_policy());
    output.extend_from_slice(b",\"cost_policy\":");
    write_policy(&mut output, spec.cost_policy());
    output.extend_from_slice(b",\"evaluation_policy\":");
    write_policy(&mut output, spec.evaluation_policy());
    output.push(b'}');
    output
}

pub fn factor_spec_id(spec: &FactorSpec) -> FactorSpecId {
    FactorSpecId::from_bytes(domain_hash(
        FACTOR_SPEC_DOMAIN,
        &canonical_factor_spec_bytes(spec),
    ))
}

fn parse_sha256(value: &str) -> Result<[u8; 32], CanonicalizationError> {
    ExpressionId::parse(value)
        .map(|identity| *identity.as_bytes())
        .map_err(|error| CanonicalizationError::Parse(error.to_string()))
}

fn domain_hash(domain: &[u8], canonical: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical);
    hasher.finalize().into()
}

fn write_registry_value_type(output: &mut Vec<u8>, value_type: &ValueType) {
    match value_type {
        ValueType::Series => output.extend_from_slice(b"\"series\""),
        ValueType::Decimal => output.extend_from_slice(b"\"decimal\""),
        ValueType::Boolean => output.extend_from_slice(b"\"boolean\""),
        ValueType::Enumeration(enum_type) => {
            output.extend_from_slice(b"{\"enumType\":\"");
            output.extend_from_slice(enum_type.as_str().as_bytes());
            output.extend_from_slice(b"\"}");
        }
    }
}

fn write_registry_argument(output: &mut Vec<u8>, argument: &super::ArgumentDefinition) {
    output.extend_from_slice(b"{\"type\":");
    write_registry_value_type(output, argument.value_type());
    if argument.literal_only {
        output.extend_from_slice(b",\"literalOnly\":true");
    }
    if let Some(decimal) = &argument.decimal {
        output.extend_from_slice(b",\"decimal\":{\"maxPrecision\":\"");
        output.extend_from_slice(decimal.max_precision.to_string().as_bytes());
        output.extend_from_slice(b"\",\"maxScale\":\"");
        output.extend_from_slice(decimal.max_scale.to_string().as_bytes());
        output.extend_from_slice(b"\",\"minimum\":\"");
        output.extend_from_slice(decimal.minimum.as_str().as_bytes());
        output.extend_from_slice(b"\",\"maximum\":\"");
        output.extend_from_slice(decimal.maximum.as_str().as_bytes());
        output.extend_from_slice(b"\"}");
    }
    output.push(b'}');
}

fn write_registry_operator(output: &mut Vec<u8>, definition: &OperatorDefinition) {
    output.extend_from_slice(b"{\"operator\":\"");
    output.extend_from_slice(definition.operator.name().as_str().as_bytes());
    output.extend_from_slice(b"\",\"operatorVersion\":\"");
    output.extend_from_slice(definition.operator.semantic_version().as_str().as_bytes());
    output.extend_from_slice(b"\",\"semanticContractSha256\":\"");
    output.extend_from_slice(definition.semantic_contract_sha256().to_string().as_bytes());
    output.extend_from_slice(b"\",\"parameters\":[");
    for (index, argument) in definition.parameters.iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        write_registry_argument(output, argument);
    }
    output.push(b']');
    if let Some(variadic) = &definition.variadic {
        output.extend_from_slice(b",\"variadic\":");
        write_registry_argument(output, variadic);
        output.extend_from_slice(b",\"minArguments\":\"");
        output.extend_from_slice(definition.minimum_arguments.to_string().as_bytes());
        output.extend_from_slice(b"\",\"maxArguments\":\"");
        output.extend_from_slice(definition.maximum_arguments.to_string().as_bytes());
        output.push(b'"');
    }
    output.extend_from_slice(b",\"outputType\":");
    write_registry_value_type(output, &definition.output_type);
    output.extend_from_slice(b",\"associative\":");
    if definition.policy.is_associative() {
        output.extend_from_slice(b"true");
    } else {
        output.extend_from_slice(b"false");
    }
    output.extend_from_slice(b",\"commutative\":");
    if definition.policy.is_commutative() {
        output.extend_from_slice(b"true");
    } else {
        output.extend_from_slice(b"false");
    }
    output.push(b'}');
}

fn compare_positive_integers(
    left: &PositiveInteger,
    right: &PositiveInteger,
) -> std::cmp::Ordering {
    left.as_str()
        .len()
        .cmp(&right.as_str().len())
        .then_with(|| left.as_str().cmp(right.as_str()))
}

fn validate_expression(
    expression: &FactorExpr,
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
) -> Result<ValueType, ValidationError> {
    validate_expression_at_stage(expression, registry, limits, ValidationStage::Normalized)
}

fn validate_submitted_expression(
    expression: &FactorExpr,
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
) -> Result<ValueType, ValidationError> {
    validate_expression_at_stage(expression, registry, limits, ValidationStage::Submitted)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ValidationStage {
    Submitted,
    Normalized,
}

fn validate_expression_at_stage(
    expression: &FactorExpr,
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
    stage: ValidationStage,
) -> Result<ValueType, ValidationError> {
    let mut nodes = 0_usize;
    let mut scalar_bytes = 0_usize;
    validate_node(
        expression,
        1,
        &mut nodes,
        &mut scalar_bytes,
        registry,
        limits,
        stage,
    )
}

fn validate_node(
    expression: &FactorExpr,
    depth: usize,
    nodes: &mut usize,
    scalar_bytes: &mut usize,
    registry: &OperatorPolicyRegistry,
    limits: ValidationLimits,
    stage: ValidationStage,
) -> Result<ValueType, ValidationError> {
    if depth > limits.max_depth() {
        return Err(ValidationError::DepthLimit {
            actual: depth,
            maximum: limits.max_depth(),
        });
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > limits.max_nodes() {
        return Err(ValidationError::NodeLimit {
            maximum: limits.max_nodes(),
        });
    }

    match expression {
        FactorExpr::Field(field) => {
            add_scalar_bytes(scalar_bytes, field.field().as_str().len(), limits)?;
            registry.field_type(field.field()).cloned().ok_or_else(|| {
                ValidationError::UnknownField {
                    field: field.field().clone(),
                }
            })
        }
        FactorExpr::Literal(Literal::Decimal(value)) => {
            add_scalar_bytes(scalar_bytes, value.as_str().len(), limits)?;
            Ok(ValueType::Decimal)
        }
        FactorExpr::Literal(Literal::Boolean(_)) => Ok(ValueType::Boolean),
        FactorExpr::Literal(Literal::Enumeration(value)) => {
            add_scalar_bytes(scalar_bytes, value.enum_type().as_str().len(), limits)?;
            add_scalar_bytes(scalar_bytes, value.value().as_str().len(), limits)?;
            if !registry.enum_contains(value.enum_type(), value.value()) {
                return Err(ValidationError::UnknownEnum {
                    enum_type: value.enum_type().clone(),
                    value: value.value().clone(),
                });
            }
            Ok(ValueType::Enumeration(value.enum_type().clone()))
        }
        FactorExpr::Call(call) => {
            add_scalar_bytes(scalar_bytes, call.operator().name().as_str().len(), limits)?;
            add_scalar_bytes(
                scalar_bytes,
                call.operator().semantic_version().as_str().len(),
                limits,
            )?;
            let definition = registry.definition_for(call.operator()).ok_or_else(|| {
                ValidationError::UnknownOperator {
                    operator: call.operator().name().clone(),
                    version: call.operator().semantic_version().clone(),
                }
            })?;
            if call.arguments().len() > limits.max_arguments() {
                return Err(ValidationError::ArgumentLimit {
                    operator: call.operator().name().clone(),
                    actual: call.arguments().len(),
                    maximum: limits.max_arguments(),
                });
            }
            let validate_signature =
                stage == ValidationStage::Normalized || !definition.policy.is_associative();
            if validate_signature
                && (call.arguments().len() < definition.minimum_arguments
                    || call.arguments().len() > definition.maximum_arguments)
            {
                return Err(ValidationError::ArityMismatch {
                    operator: call.operator().name().clone(),
                    minimum: definition.minimum_arguments,
                    maximum: definition.maximum_arguments,
                    actual: call.arguments().len(),
                });
            }
            for (index, child) in call.arguments().iter().enumerate() {
                let actual = validate_node(
                    child,
                    depth + 1,
                    nodes,
                    scalar_bytes,
                    registry,
                    limits,
                    stage,
                )?;
                if validate_signature {
                    validate_argument(definition, call, index, child, &actual)?;
                }
            }
            Ok(definition.output_type().clone())
        }
    }
}

fn validate_argument(
    definition: &OperatorDefinition,
    call: &OperatorCall,
    index: usize,
    expression: &FactorExpr,
    actual: &ValueType,
) -> Result<(), ValidationError> {
    let expected =
        definition
            .argument_definition(index)
            .ok_or_else(|| ValidationError::ArityMismatch {
                operator: call.operator().name().clone(),
                minimum: definition.minimum_arguments,
                maximum: definition.maximum_arguments,
                actual: call.arguments().len(),
            })?;
    if expected.value_type() != actual {
        return Err(ValidationError::TypeMismatch {
            operator: call.operator().name().clone(),
            index,
            expected: expected.value_type().label(),
            actual: actual.label(),
        });
    }
    if expected.is_literal_only() && !is_literal_of_type(expression, expected.value_type()) {
        return Err(ValidationError::LiteralRequired {
            operator: call.operator().name().clone(),
            index,
        });
    }
    if let (Some(constraints), FactorExpr::Literal(Literal::Decimal(value))) =
        (expected.decimal_constraints(), expression)
    {
        constraints.validate(value)?;
    }
    Ok(())
}

fn is_literal_of_type(expression: &FactorExpr, value_type: &ValueType) -> bool {
    matches!(
        (expression, value_type),
        (FactorExpr::Literal(Literal::Decimal(_)), ValueType::Decimal)
            | (FactorExpr::Literal(Literal::Boolean(_)), ValueType::Boolean)
            | (
                FactorExpr::Literal(Literal::Enumeration(_)),
                ValueType::Enumeration(_)
            )
    )
}

fn add_scalar_bytes(
    current: &mut usize,
    additional: usize,
    limits: ValidationLimits,
) -> Result<(), ValidationError> {
    *current = (*current).saturating_add(additional);
    validate_canonical_size(*current, limits)
}

fn validate_canonical_size(actual: usize, limits: ValidationLimits) -> Result<(), ValidationError> {
    if actual > limits.max_canonical_bytes() {
        Err(ValidationError::CanonicalByteLimit {
            actual,
            maximum: limits.max_canonical_bytes(),
        })
    } else {
        Ok(())
    }
}

fn normalize_expression(expression: &FactorExpr, registry: &OperatorPolicyRegistry) -> FactorExpr {
    let FactorExpr::Call(call) = expression else {
        return expression.clone();
    };

    let mut arguments: Vec<_> = call
        .arguments()
        .iter()
        .map(|child| normalize_expression(child, registry))
        .collect();

    let policy = registry
        .policy_for(call.operator())
        .expect("validation resolves every operator before normalization");
    if policy.is_associative() {
        arguments = arguments
            .into_iter()
            .flat_map(|child| match child {
                FactorExpr::Call(nested) if nested.operator() == call.operator() => {
                    nested.arguments
                }
                other => vec![other],
            })
            .collect();
    }
    if policy.is_commutative() {
        let mut keyed_arguments: Vec<_> = arguments
            .into_iter()
            .map(|argument| {
                let mut bytes = Vec::new();
                write_expression(&mut bytes, &argument);
                (bytes, argument)
            })
            .collect();
        keyed_arguments.sort_by(|left, right| left.0.cmp(&right.0));
        arguments = keyed_arguments
            .into_iter()
            .map(|(_, argument)| argument)
            .collect();
    }

    FactorExpr::Call(OperatorCall::new(call.operator().clone(), arguments))
}

fn write_expression(output: &mut Vec<u8>, expression: &FactorExpr) {
    match expression {
        FactorExpr::Field(field) => {
            output.extend_from_slice(b"{\"node\":\"field\",\"field\":\"");
            output.extend_from_slice(field.field().as_str().as_bytes());
            output.extend_from_slice(b"\"}");
        }
        FactorExpr::Literal(Literal::Decimal(value)) => {
            output.extend_from_slice(b"{\"node\":\"decimal\",\"value\":\"");
            output.extend_from_slice(value.as_str().as_bytes());
            output.extend_from_slice(b"\"}");
        }
        FactorExpr::Literal(Literal::Boolean(value)) => {
            output.extend_from_slice(b"{\"node\":\"boolean\",\"value\":");
            output.extend_from_slice(if *value { b"true" } else { b"false" });
            output.push(b'}');
        }
        FactorExpr::Literal(Literal::Enumeration(value)) => write_enum(output, value),
        FactorExpr::Call(call) => {
            output.extend_from_slice(b"{\"node\":\"call\",\"operator\":\"");
            output.extend_from_slice(call.operator().name().as_str().as_bytes());
            output.extend_from_slice(b"\",\"operator_version\":\"");
            output.extend_from_slice(call.operator().semantic_version().as_str().as_bytes());
            output.extend_from_slice(b"\",\"arguments\":[");
            for (index, argument) in call.arguments().iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_expression(output, argument);
            }
            output.extend_from_slice(b"]}");
        }
    }
}

fn write_enum(output: &mut Vec<u8>, value: &EnumLiteral) {
    output.extend_from_slice(b"{\"node\":\"enum\",\"enum_type\":\"");
    output.extend_from_slice(value.enum_type().as_str().as_bytes());
    output.extend_from_slice(b"\",\"value\":\"");
    output.extend_from_slice(value.value().as_str().as_bytes());
    output.extend_from_slice(b"\"}");
}

fn write_policy(output: &mut Vec<u8>, policy: &PolicyRef) {
    output.extend_from_slice(b"{\"policy_id\":\"");
    output.extend_from_slice(policy.policy_id().as_str().as_bytes());
    output.extend_from_slice(b"\",\"revision\":\"");
    output.extend_from_slice(policy.revision().as_str().as_bytes());
    output.extend_from_slice(b"\",\"sha256\":\"sha256:");
    output.extend_from_slice(encode_digest(policy.sha256()).as_bytes());
    output.extend_from_slice(b"\"}");
}
