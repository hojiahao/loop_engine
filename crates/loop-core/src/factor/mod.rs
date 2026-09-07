//! Provider-neutral factor expressions and reproducible content identities.
//!
//! Wire encodings are deliberately not used for identities. This module owns
//! the closed v1 domain model, resource validation, normalization, a dedicated
//! canonical JSON writer, and version-separated SHA-256 identities.

mod canonical;
mod semantic;

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

pub use canonical::{
    bind_factor_spec, canonical_expression_bytes, canonical_factor_spec_bytes,
    canonical_operator_registry_bytes, canonicalize_expression, expression_id, factor_spec_id,
    operator_registry_id, parse_canonical_expression, parse_canonical_factor_spec,
};
pub use semantic::{
    AlignmentPolicy, NullPolicy, NumericPolicy, OPERATOR_SEMANTIC_CONTRACT_SCHEMA,
    OperatorSemanticContract, SemanticContractResolver, TiePolicy, WindowPolicy,
    parse_canonical_operator_semantic_contract, semantic_contract_sha256,
};

const MAX_IDENTIFIER_BYTES: usize = 128;

/// A dot-qualified ASCII identifier with one stable spelling.
///
/// The grammar is `[a-z][a-z0-9_]*(.[a-z][a-z0-9_]*)*` and the length is 1
/// through 128 bytes. This narrow alphabet removes Unicode normalization and
/// JSON escaping from the identity format.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

impl Identifier {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_identifier(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Identifier {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for Identifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Identifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

fn validate_identifier(value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(IdentifierError::TooLong {
            actual: value.len(),
            maximum: MAX_IDENTIFIER_BYTES,
        });
    }

    for (segment_index, segment) in value.split('.').enumerate() {
        if segment.is_empty() {
            return Err(IdentifierError::EmptySegment { segment_index });
        }
        let mut bytes = segment.bytes();
        let first = bytes.next().expect("empty segments are rejected above");
        if !first.is_ascii_lowercase() {
            return Err(IdentifierError::InvalidSegmentStart {
                segment_index,
                byte: first,
            });
        }
        if let Some((byte_index, byte)) = bytes.enumerate().find(|(_, byte)| {
            !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        }) {
            return Err(IdentifierError::InvalidByte {
                segment_index,
                byte_index: byte_index + 1,
                byte,
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentifierError {
    #[error("identifier must not be empty")]
    Empty,
    #[error("identifier has {actual} bytes; the maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },
    #[error("identifier contains an empty segment at index {segment_index}")]
    EmptySegment { segment_index: usize },
    #[error(
        "identifier segment {segment_index} must start with an ASCII lowercase letter, found byte 0x{byte:02x}"
    )]
    InvalidSegmentStart { segment_index: usize, byte: u8 },
    #[error(
        "identifier segment {segment_index} contains invalid byte 0x{byte:02x} at offset {byte_index}"
    )]
    InvalidByte {
        segment_index: usize,
        byte_index: usize,
        byte: u8,
    },
}

/// An exact base-ten number with a single accepted textual representation.
///
/// Exponents, leading/trailing zeroes, a leading plus sign, and negative zero
/// are rejected. The value is never converted through binary floating point.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalDecimal(String);

impl CanonicalDecimal {
    pub fn new(value: impl Into<String>) -> Result<Self, DecimalError> {
        let value = value.into();
        validate_decimal(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CanonicalDecimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CanonicalDecimal {
    type Err = DecimalError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for CanonicalDecimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CanonicalDecimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

fn validate_decimal(value: &str) -> Result<(), DecimalError> {
    if value.is_empty() {
        return Err(DecimalError::Empty);
    }

    let (negative, magnitude) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    if magnitude.is_empty() {
        return Err(DecimalError::InvalidSyntax);
    }

    let mut parts = magnitude.split('.');
    let integer = parts.next().expect("split always returns one element");
    let fraction = parts.next();
    if parts.next().is_some() || integer.is_empty() {
        return Err(DecimalError::InvalidSyntax);
    }
    if !integer.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DecimalError::InvalidSyntax);
    }
    if integer.len() > 1 && integer.starts_with('0') {
        return Err(DecimalError::LeadingZero);
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(DecimalError::InvalidSyntax);
        }
        if fraction.ends_with('0') {
            return Err(DecimalError::TrailingFractionalZero);
        }
    }
    if negative && integer == "0" && fraction.is_none() {
        return Err(DecimalError::NegativeZero);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecimalError {
    #[error("decimal must not be empty")]
    Empty,
    #[error("decimal syntax is not canonical fixed-point base ten")]
    InvalidSyntax,
    #[error("decimal integer part contains a leading zero")]
    LeadingZero,
    #[error("decimal fractional part contains a trailing zero")]
    TrailingFractionalZero,
    #[error("negative zero is not canonical")]
    NegativeZero,
}

/// A normalized positive u64 represented as decimal text on the wire and in
/// canonical JSON.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PositiveInteger(String);

impl PositiveInteger {
    pub fn new(value: impl Into<String>) -> Result<Self, PositiveIntegerError> {
        let value = value.into();
        if value.is_empty()
            || value.starts_with('0')
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || value.len() > 20
            || value.parse::<u64>().is_err()
        {
            return Err(PositiveIntegerError);
        }
        Ok(Self(value))
    }

    pub fn from_u64(value: u64) -> Result<Self, PositiveIntegerError> {
        Self::new(value.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PositiveInteger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PositiveInteger {
    type Err = PositiveIntegerError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for PositiveInteger {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PositiveInteger {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("positive integer must be canonical decimal in 1..=18446744073709551615")]
pub struct PositiveIntegerError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldRef {
    field: Identifier,
}

impl FieldRef {
    pub fn new(field: Identifier) -> Self {
        Self { field }
    }

    pub fn field(&self) -> &Identifier {
        &self.field
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumLiteral {
    enum_type: Identifier,
    value: Identifier,
}

impl EnumLiteral {
    pub fn new(enum_type: Identifier, value: Identifier) -> Self {
        Self { enum_type, value }
    }

    pub fn enum_type(&self) -> &Identifier {
        &self.enum_type
    }

    pub fn value(&self) -> &Identifier {
        &self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Literal {
    Decimal(CanonicalDecimal),
    Boolean(bool),
    Enumeration(EnumLiteral),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OperatorRef {
    name: Identifier,
    semantic_version: PositiveInteger,
}

impl OperatorRef {
    pub fn new(name: Identifier, semantic_version: PositiveInteger) -> Self {
        Self {
            name,
            semantic_version,
        }
    }

    pub fn name(&self) -> &Identifier {
        &self.name
    }

    pub fn semantic_version(&self) -> &PositiveInteger {
        &self.semantic_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorCall {
    operator: OperatorRef,
    arguments: Vec<FactorExpr>,
}

impl OperatorCall {
    pub fn new(operator: OperatorRef, arguments: Vec<FactorExpr>) -> Self {
        Self {
            operator,
            arguments,
        }
    }

    pub fn operator(&self) -> &OperatorRef {
        &self.operator
    }

    pub fn arguments(&self) -> &[FactorExpr] {
        &self.arguments
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorExpr {
    Field(FieldRef),
    Literal(Literal),
    Call(OperatorCall),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorDirection {
    HigherIsBetter,
    LowerIsBetter,
}

/// Policy IDs have a slightly wider grammar than domain identifiers because
/// immutable policy registries commonly use dots and hyphens.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PolicyId(String);

impl PolicyId {
    pub fn new(value: impl Into<String>) -> Result<Self, PolicyIdError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_IDENTIFIER_BYTES
            && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            && value.bytes().skip(1).all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'.' | b'-')
            });
        if !valid {
            return Err(PolicyIdError);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PolicyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PolicyId {
    type Err = PolicyIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for PolicyId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PolicyId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("policy ID must match [a-z][a-z0-9_.-]{{0,127}}")]
pub struct PolicyIdError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRef {
    policy_id: PolicyId,
    revision: PositiveInteger,
    sha256: [u8; 32],
}

impl PolicyRef {
    pub fn new(policy_id: PolicyId, revision: PositiveInteger, sha256: [u8; 32]) -> Self {
        Self {
            policy_id,
            revision,
            sha256,
        }
    }

    pub fn policy_id(&self) -> &PolicyId {
        &self.policy_id
    }

    pub fn revision(&self) -> &PositiveInteger {
        &self.revision
    }

    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }
}

/// An unbound specification received from a wire DTO or configuration file.
///
/// A draft is intentionally not hashable. It becomes a [`FactorSpec`] only
/// after [`bind_factor_spec`] verifies its canonical expression bytes and
/// confirms that the registry resolves the expression root to `series`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactorSpecDraft {
    expression_id: ExpressionId,
    operator_registry_sha256: [u8; 32],
    direction: FactorDirection,
    universe_policy: PolicyRef,
    data_policy: PolicyRef,
    calendar_policy: PolicyRef,
    preprocess_policy: PolicyRef,
    neutralization_policy: PolicyRef,
    portfolio_policy: PolicyRef,
    execution_policy: PolicyRef,
    cost_policy: PolicyRef,
    evaluation_policy: PolicyRef,
}

impl FactorSpecDraft {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        expression_id: ExpressionId,
        operator_registry_sha256: [u8; 32],
        direction: FactorDirection,
        universe_policy: PolicyRef,
        data_policy: PolicyRef,
        calendar_policy: PolicyRef,
        preprocess_policy: PolicyRef,
        neutralization_policy: PolicyRef,
        portfolio_policy: PolicyRef,
        execution_policy: PolicyRef,
        cost_policy: PolicyRef,
        evaluation_policy: PolicyRef,
    ) -> Self {
        Self {
            expression_id,
            operator_registry_sha256,
            direction,
            universe_policy,
            data_policy,
            calendar_policy,
            preprocess_policy,
            neutralization_policy,
            portfolio_policy,
            execution_policy,
            cost_policy,
            evaluation_policy,
        }
    }

    pub fn expression_id(&self) -> ExpressionId {
        self.expression_id
    }

    pub fn operator_registry_sha256(&self) -> &[u8; 32] {
        &self.operator_registry_sha256
    }

    pub fn direction(&self) -> FactorDirection {
        self.direction
    }

    pub fn universe_policy(&self) -> &PolicyRef {
        &self.universe_policy
    }

    pub fn data_policy(&self) -> &PolicyRef {
        &self.data_policy
    }

    pub fn calendar_policy(&self) -> &PolicyRef {
        &self.calendar_policy
    }

    pub fn preprocess_policy(&self) -> &PolicyRef {
        &self.preprocess_policy
    }

    pub fn neutralization_policy(&self) -> &PolicyRef {
        &self.neutralization_policy
    }

    pub fn portfolio_policy(&self) -> &PolicyRef {
        &self.portfolio_policy
    }

    pub fn execution_policy(&self) -> &PolicyRef {
        &self.execution_policy
    }

    pub fn cost_policy(&self) -> &PolicyRef {
        &self.cost_policy
    }

    pub fn evaluation_policy(&self) -> &PolicyRef {
        &self.evaluation_policy
    }
}

/// A specification cryptographically bound to a canonical, series-valued
/// expression under one immutable operator registry.
///
/// This type has no public constructor or deserialization implementation.
/// Hashing APIs accept this verified type rather than [`FactorSpecDraft`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorSpec {
    draft: FactorSpecDraft,
    canonical_expression: FactorExpr,
}

impl FactorSpec {
    fn from_verified(draft: FactorSpecDraft, canonical_expression: FactorExpr) -> Self {
        Self {
            draft,
            canonical_expression,
        }
    }

    pub fn expression_id(&self) -> ExpressionId {
        self.draft.expression_id()
    }

    pub fn operator_registry_sha256(&self) -> &[u8; 32] {
        self.draft.operator_registry_sha256()
    }

    pub fn direction(&self) -> FactorDirection {
        self.draft.direction()
    }

    pub fn universe_policy(&self) -> &PolicyRef {
        self.draft.universe_policy()
    }

    pub fn data_policy(&self) -> &PolicyRef {
        self.draft.data_policy()
    }

    pub fn calendar_policy(&self) -> &PolicyRef {
        self.draft.calendar_policy()
    }

    pub fn preprocess_policy(&self) -> &PolicyRef {
        self.draft.preprocess_policy()
    }

    pub fn neutralization_policy(&self) -> &PolicyRef {
        self.draft.neutralization_policy()
    }

    pub fn portfolio_policy(&self) -> &PolicyRef {
        self.draft.portfolio_policy()
    }

    pub fn execution_policy(&self) -> &PolicyRef {
        self.draft.execution_policy()
    }

    pub fn cost_policy(&self) -> &PolicyRef {
        self.draft.cost_policy()
    }

    pub fn evaluation_policy(&self) -> &PolicyRef {
        self.draft.evaluation_policy()
    }

    pub fn canonical_expression(&self) -> &FactorExpr {
        &self.canonical_expression
    }
}

/// Closed value types used by the immutable operator-registry snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    Series,
    Decimal,
    Boolean,
    Enumeration(Identifier),
}

impl ValueType {
    fn label(&self) -> String {
        match self {
            Self::Series => "series".to_owned(),
            Self::Decimal => "decimal".to_owned(),
            Self::Boolean => "boolean".to_owned(),
            Self::Enumeration(enum_type) => format!("enum:{enum_type}"),
        }
    }
}

/// Exact fixed-point limits for one decimal argument position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalConstraints {
    max_precision: usize,
    max_scale: usize,
    minimum: CanonicalDecimal,
    maximum: CanonicalDecimal,
}

impl DecimalConstraints {
    pub const HARD_MAX_PRECISION: usize = 4_096;

    pub fn new(
        max_precision: usize,
        max_scale: usize,
        minimum: CanonicalDecimal,
        maximum: CanonicalDecimal,
    ) -> Result<Self, RegistryError> {
        if !(1..=Self::HARD_MAX_PRECISION).contains(&max_precision) {
            return Err(RegistryError::InvalidDecimalConstraints(
                "max_precision must be in 1..=4096".to_owned(),
            ));
        }
        if max_scale > max_precision {
            return Err(RegistryError::InvalidDecimalConstraints(
                "max_scale cannot exceed max_precision".to_owned(),
            ));
        }
        let constraints = Self {
            max_precision,
            max_scale,
            minimum,
            maximum,
        };
        constraints.validate_shape(&constraints.minimum)?;
        constraints.validate_shape(&constraints.maximum)?;
        if compare_canonical_decimals(&constraints.minimum, &constraints.maximum).is_gt() {
            return Err(RegistryError::InvalidDecimalConstraints(
                "minimum cannot exceed maximum".to_owned(),
            ));
        }
        Ok(constraints)
    }

    pub fn max_precision(&self) -> usize {
        self.max_precision
    }

    pub fn max_scale(&self) -> usize {
        self.max_scale
    }

    pub fn minimum(&self) -> &CanonicalDecimal {
        &self.minimum
    }

    pub fn maximum(&self) -> &CanonicalDecimal {
        &self.maximum
    }

    fn validate(&self, value: &CanonicalDecimal) -> Result<(), ValidationError> {
        self.validate_shape(value)
            .map_err(|error| ValidationError::DecimalConstraint {
                value: value.clone(),
                reason: error.to_string(),
            })?;
        if compare_canonical_decimals(value, &self.minimum).is_lt() {
            return Err(ValidationError::DecimalConstraint {
                value: value.clone(),
                reason: format!("value is below minimum {}", self.minimum),
            });
        }
        if compare_canonical_decimals(value, &self.maximum).is_gt() {
            return Err(ValidationError::DecimalConstraint {
                value: value.clone(),
                reason: format!("value is above maximum {}", self.maximum),
            });
        }
        Ok(())
    }

    fn validate_shape(&self, value: &CanonicalDecimal) -> Result<(), RegistryError> {
        let (precision, scale) = decimal_precision_and_scale(value);
        if precision > self.max_precision {
            return Err(RegistryError::InvalidDecimalConstraints(format!(
                "decimal precision {precision} exceeds {}",
                self.max_precision
            )));
        }
        if scale > self.max_scale {
            return Err(RegistryError::InvalidDecimalConstraints(format!(
                "decimal scale {scale} exceeds {}",
                self.max_scale
            )));
        }
        Ok(())
    }
}

fn decimal_precision_and_scale(value: &CanonicalDecimal) -> (usize, usize) {
    let unsigned = value.as_str().strip_prefix('-').unwrap_or(value.as_str());
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    (integer.len() + fraction.len(), fraction.len())
}

fn compare_canonical_decimals(
    left: &CanonicalDecimal,
    right: &CanonicalDecimal,
) -> std::cmp::Ordering {
    fn split(value: &CanonicalDecimal) -> (bool, &str, &str) {
        let negative = value.as_str().starts_with('-');
        let unsigned = value.as_str().strip_prefix('-').unwrap_or(value.as_str());
        let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        (negative, integer, fraction)
    }
    let (left_negative, left_integer, left_fraction) = split(left);
    let (right_negative, right_integer, right_fraction) = split(right);
    if left_negative != right_negative {
        return if left_negative {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        };
    }
    let magnitude = left_integer
        .len()
        .cmp(&right_integer.len())
        .then_with(|| left_integer.cmp(right_integer))
        .then_with(|| {
            let width = left_fraction.len().max(right_fraction.len());
            let mut left_padded = left_fraction.as_bytes().to_vec();
            let mut right_padded = right_fraction.as_bytes().to_vec();
            left_padded.resize(width, b'0');
            right_padded.resize(width, b'0');
            left_padded.cmp(&right_padded)
        });
    if left_negative {
        magnitude.reverse()
    } else {
        magnitude
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentDefinition {
    value_type: ValueType,
    literal_only: bool,
    decimal: Option<DecimalConstraints>,
}

impl ArgumentDefinition {
    pub fn new(
        value_type: ValueType,
        literal_only: bool,
        decimal: Option<DecimalConstraints>,
    ) -> Result<Self, RegistryError> {
        if matches!(value_type, ValueType::Decimal) != decimal.is_some() {
            return Err(RegistryError::InvalidArgumentDefinition(
                "decimal arguments require constraints and non-decimal arguments forbid them"
                    .to_owned(),
            ));
        }
        Ok(Self {
            value_type,
            literal_only,
            decimal,
        })
    }

    pub fn series() -> Self {
        Self {
            value_type: ValueType::Series,
            literal_only: false,
            decimal: None,
        }
    }

    pub fn boolean_literal() -> Self {
        Self {
            value_type: ValueType::Boolean,
            literal_only: true,
            decimal: None,
        }
    }

    pub fn enumeration_literal(enum_type: Identifier) -> Self {
        Self {
            value_type: ValueType::Enumeration(enum_type),
            literal_only: true,
            decimal: None,
        }
    }

    pub fn decimal_literal(constraints: DecimalConstraints) -> Self {
        Self {
            value_type: ValueType::Decimal,
            literal_only: true,
            decimal: Some(constraints),
        }
    }

    pub fn value_type(&self) -> &ValueType {
        &self.value_type
    }

    pub fn is_literal_only(&self) -> bool {
        self.literal_only
    }

    pub fn decimal_constraints(&self) -> Option<&DecimalConstraints> {
        self.decimal.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperatorPolicy {
    commutative: bool,
    associative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorDefinition {
    operator: OperatorRef,
    parameters: Vec<ArgumentDefinition>,
    variadic: Option<ArgumentDefinition>,
    minimum_arguments: usize,
    maximum_arguments: usize,
    output_type: ValueType,
    policy: OperatorPolicy,
    semantic_contract_sha256: SemanticContractId,
}

impl OperatorDefinition {
    pub fn fixed(
        operator: OperatorRef,
        parameters: Vec<ArgumentDefinition>,
        output_type: ValueType,
        policy: OperatorPolicy,
        semantic_contract_sha256: SemanticContractId,
    ) -> Result<Self, RegistryError> {
        let arity = parameters.len();
        Self::build(
            operator,
            parameters,
            None,
            arity,
            arity,
            output_type,
            policy,
            semantic_contract_sha256,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn variadic(
        operator: OperatorRef,
        parameters: Vec<ArgumentDefinition>,
        variadic: ArgumentDefinition,
        minimum_arguments: usize,
        maximum_arguments: usize,
        output_type: ValueType,
        policy: OperatorPolicy,
        semantic_contract_sha256: SemanticContractId,
    ) -> Result<Self, RegistryError> {
        Self::build(
            operator,
            parameters,
            Some(variadic),
            minimum_arguments,
            maximum_arguments,
            output_type,
            policy,
            semantic_contract_sha256,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        operator: OperatorRef,
        parameters: Vec<ArgumentDefinition>,
        variadic: Option<ArgumentDefinition>,
        minimum_arguments: usize,
        maximum_arguments: usize,
        output_type: ValueType,
        policy: OperatorPolicy,
        semantic_contract_sha256: SemanticContractId,
    ) -> Result<Self, RegistryError> {
        if parameters.len() > ValidationLimits::HARD_MAX_ARGUMENTS
            || maximum_arguments > ValidationLimits::HARD_MAX_ARGUMENTS
            || minimum_arguments < parameters.len()
            || maximum_arguments < minimum_arguments
            || (variadic.is_some() && minimum_arguments == 0)
        {
            return Err(RegistryError::InvalidOperatorDefinition(
                "operator arity is inconsistent with the v1 limits".to_owned(),
            ));
        }
        if variadic.is_none()
            && (minimum_arguments != parameters.len() || maximum_arguments != parameters.len())
        {
            return Err(RegistryError::InvalidOperatorDefinition(
                "fixed signatures must use their exact parameter count".to_owned(),
            ));
        }
        if policy.is_associative() && variadic.is_none() {
            return Err(RegistryError::InvalidOperatorDefinition(
                "associative operators require a variadic signature".to_owned(),
            ));
        }
        if policy.is_associative() || policy.is_commutative() {
            let definitions = parameters.iter().chain(variadic.iter());
            for definition in definitions {
                if definition.value_type() != &output_type {
                    return Err(RegistryError::InvalidOperatorDefinition(
                        "rewritable operators require homogeneous input and output types"
                            .to_owned(),
                    ));
                }
            }
            let mut definitions = parameters.iter().chain(variadic.iter());
            if let Some(first) = definitions.next()
                && definitions.any(|definition| definition != first)
            {
                return Err(RegistryError::InvalidOperatorDefinition(
                    "rewritable operators require identical argument rules".to_owned(),
                ));
            }
        }
        Ok(Self {
            operator,
            parameters,
            variadic,
            minimum_arguments,
            maximum_arguments,
            output_type,
            policy,
            semantic_contract_sha256,
        })
    }

    pub fn operator(&self) -> &OperatorRef {
        &self.operator
    }

    pub fn policy(&self) -> OperatorPolicy {
        self.policy
    }

    pub fn output_type(&self) -> &ValueType {
        &self.output_type
    }

    pub fn semantic_contract_sha256(&self) -> SemanticContractId {
        self.semantic_contract_sha256
    }

    fn argument_definition(&self, index: usize) -> Option<&ArgumentDefinition> {
        self.parameters.get(index).or(self.variadic.as_ref())
    }
}

impl OperatorPolicy {
    pub const ORDERED: Self = Self::new(false, false);
    pub const COMMUTATIVE: Self = Self::new(true, false);
    pub const ASSOCIATIVE: Self = Self::new(false, true);
    pub const COMMUTATIVE_ASSOCIATIVE: Self = Self::new(true, true);

    pub const fn new(commutative: bool, associative: bool) -> Self {
        Self {
            commutative,
            associative,
        }
    }

    pub const fn is_commutative(self) -> bool {
        self.commutative
    }

    pub const fn is_associative(self) -> bool {
        self.associative
    }
}

/// Algebraic rules keyed by exact operator name and semantic version.
///
/// Every call must resolve to an entry. Algebraic properties never carry from
/// one semantic version to another.
#[derive(Debug, Clone)]
pub struct OperatorPolicyRegistry {
    fields: BTreeMap<Identifier, ValueType>,
    enums: BTreeMap<Identifier, std::collections::BTreeSet<Identifier>>,
    operators: BTreeMap<OperatorRef, OperatorDefinition>,
    semantic_contracts: BTreeMap<SemanticContractId, OperatorSemanticContract>,
    identity: OperatorRegistryId,
}

#[derive(Debug, Clone, Default)]
pub struct OperatorRegistryBuilder {
    fields: BTreeMap<Identifier, ValueType>,
    enums: BTreeMap<Identifier, std::collections::BTreeSet<Identifier>>,
    operators: BTreeMap<OperatorRef, OperatorDefinition>,
}

impl OperatorRegistryBuilder {
    pub fn new() -> Self {
        Self {
            fields: BTreeMap::new(),
            enums: BTreeMap::new(),
            operators: BTreeMap::new(),
        }
    }

    pub fn register_field(
        &mut self,
        field: Identifier,
        output_type: ValueType,
    ) -> Result<(), RegistryError> {
        self.validate_registered_type(&output_type)?;
        if self.fields.contains_key(&field) {
            return Err(RegistryError::DuplicateField { field });
        }
        self.fields.insert(field, output_type);
        Ok(())
    }

    pub fn register_enum(
        &mut self,
        enum_type: Identifier,
        values: impl IntoIterator<Item = Identifier>,
    ) -> Result<(), RegistryError> {
        let mut unique_values = std::collections::BTreeSet::new();
        for value in values {
            if !unique_values.insert(value.clone()) {
                return Err(RegistryError::DuplicateEnumValue { enum_type, value });
            }
        }
        if unique_values.is_empty() {
            return Err(RegistryError::EmptyEnum { enum_type });
        }
        if self.enums.contains_key(&enum_type) {
            return Err(RegistryError::DuplicateEnum { enum_type });
        }
        self.enums.insert(enum_type, unique_values);
        Ok(())
    }

    pub fn register_operator(
        &mut self,
        definition: OperatorDefinition,
    ) -> Result<(), RegistryError> {
        self.validate_registered_type(definition.output_type())?;
        for argument in definition
            .parameters
            .iter()
            .chain(definition.variadic.iter())
        {
            self.validate_registered_type(argument.value_type())?;
        }
        let operator = definition.operator().clone();
        if self.operators.contains_key(&operator) {
            return Err(RegistryError::DuplicateOperator { operator });
        }
        self.operators.insert(operator, definition);
        Ok(())
    }

    pub fn build(
        self,
        resolver: &impl SemanticContractResolver,
    ) -> Result<OperatorPolicyRegistry, RegistryError> {
        let mut semantic_contracts = BTreeMap::new();
        for (operator, definition) in &self.operators {
            let identity = definition.semantic_contract_sha256();
            let contract = semantic::resolve_semantic_contract(operator, identity, resolver)?;
            semantic_contracts.insert(identity, contract);
        }
        let mut registry = OperatorPolicyRegistry {
            fields: self.fields,
            enums: self.enums,
            operators: self.operators,
            semantic_contracts,
            identity: OperatorRegistryId::from_bytes([0; 32]),
        };
        registry.identity = operator_registry_id(&registry);
        Ok(registry)
    }

    fn validate_registered_type(&self, value_type: &ValueType) -> Result<(), RegistryError> {
        if let ValueType::Enumeration(enum_type) = value_type
            && !self.enums.contains_key(enum_type)
        {
            return Err(RegistryError::UnknownEnumType {
                enum_type: enum_type.clone(),
            });
        }
        Ok(())
    }
}

impl OperatorPolicyRegistry {
    pub fn identity(&self) -> OperatorRegistryId {
        self.identity
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical_operator_registry_bytes(self)
    }

    pub fn policy_for(&self, operator: &OperatorRef) -> Option<OperatorPolicy> {
        self.operators.get(operator).map(OperatorDefinition::policy)
    }

    pub fn definition_for(&self, operator: &OperatorRef) -> Option<&OperatorDefinition> {
        self.operators.get(operator)
    }

    pub fn semantic_contract_for(
        &self,
        operator: &OperatorRef,
    ) -> Option<&OperatorSemanticContract> {
        let identity = self.operators.get(operator)?.semantic_contract_sha256();
        self.semantic_contracts.get(&identity)
    }

    pub fn semantic_contract_by_id(
        &self,
        identity: SemanticContractId,
    ) -> Option<&OperatorSemanticContract> {
        self.semantic_contracts.get(&identity)
    }

    pub fn field_type(&self, field: &Identifier) -> Option<&ValueType> {
        self.fields.get(field)
    }

    pub fn enum_contains(&self, enum_type: &Identifier, value: &Identifier) -> bool {
        self.enums
            .get(enum_type)
            .is_some_and(|values| values.contains(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error("field {field} is already registered")]
    DuplicateField { field: Identifier },
    #[error("enum {enum_type} is already registered")]
    DuplicateEnum { enum_type: Identifier },
    #[error("enum {enum_type} must contain at least one value")]
    EmptyEnum { enum_type: Identifier },
    #[error("enum {enum_type} contains duplicate value {value}")]
    DuplicateEnumValue {
        enum_type: Identifier,
        value: Identifier,
    },
    #[error("value type references unregistered enum {enum_type}")]
    UnknownEnumType { enum_type: Identifier },
    #[error("operator {operator:?} already has a policy")]
    DuplicateOperator { operator: OperatorRef },
    #[error("invalid decimal constraints: {0}")]
    InvalidDecimalConstraints(String),
    #[error("invalid argument definition: {0}")]
    InvalidArgumentDefinition(String),
    #[error("invalid operator definition: {0}")]
    InvalidOperatorDefinition(String),
    #[error("semantic contract {identity} was not resolved")]
    SemanticContractNotFound { identity: SemanticContractId },
    #[error("invalid semantic contract: {0}")]
    InvalidSemanticContract(String),
    #[error("semantic contract bytes are valid JSON but not canonical v1 bytes")]
    NonCanonicalSemanticContract,
    #[error("semantic contract digest {claimed} does not match content digest {computed}")]
    SemanticContractDigestMismatch {
        claimed: SemanticContractId,
        computed: SemanticContractId,
    },
    #[error("semantic contract names {actual:?}, expected {expected:?}")]
    SemanticContractOperatorMismatch {
        expected: OperatorRef,
        actual: OperatorRef,
    },
}

/// Deployments may lower these ceilings but cannot exceed the v1 profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationLimits {
    max_depth: usize,
    max_nodes: usize,
    max_arguments: usize,
    max_canonical_bytes: usize,
}

impl ValidationLimits {
    pub const HARD_MAX_DEPTH: usize = 64;
    pub const HARD_MAX_NODES: usize = 4_096;
    pub const HARD_MAX_ARGUMENTS: usize = 1_024;
    pub const HARD_MAX_CANONICAL_BYTES: usize = 256 * 1_024;

    pub fn new(
        max_depth: usize,
        max_nodes: usize,
        max_arguments: usize,
        max_canonical_bytes: usize,
    ) -> Result<Self, LimitConfigurationError> {
        validate_limit("max_depth", max_depth, Self::HARD_MAX_DEPTH)?;
        validate_limit("max_nodes", max_nodes, Self::HARD_MAX_NODES)?;
        validate_limit("max_arguments", max_arguments, Self::HARD_MAX_ARGUMENTS)?;
        validate_limit(
            "max_canonical_bytes",
            max_canonical_bytes,
            Self::HARD_MAX_CANONICAL_BYTES,
        )?;
        Ok(Self {
            max_depth,
            max_nodes,
            max_arguments,
            max_canonical_bytes,
        })
    }

    pub fn max_depth(self) -> usize {
        self.max_depth
    }

    pub fn max_nodes(self) -> usize {
        self.max_nodes
    }

    pub fn max_arguments(self) -> usize {
        self.max_arguments
    }

    pub fn max_canonical_bytes(self) -> usize {
        self.max_canonical_bytes
    }
}

impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            max_depth: Self::HARD_MAX_DEPTH,
            max_nodes: Self::HARD_MAX_NODES,
            max_arguments: Self::HARD_MAX_ARGUMENTS,
            max_canonical_bytes: Self::HARD_MAX_CANONICAL_BYTES,
        }
    }
}

fn validate_limit(
    name: &'static str,
    actual: usize,
    maximum: usize,
) -> Result<(), LimitConfigurationError> {
    if (1..=maximum).contains(&actual) {
        Ok(())
    } else {
        Err(LimitConfigurationError {
            name,
            actual,
            maximum,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{name}={actual} is outside the supported range 1..={maximum}")]
pub struct LimitConfigurationError {
    pub name: &'static str,
    pub actual: usize,
    pub maximum: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("expression depth {actual} exceeds limit {maximum}")]
    DepthLimit { actual: usize, maximum: usize },
    #[error("expression node count exceeds limit {maximum}")]
    NodeLimit { maximum: usize },
    #[error("operator {operator} has {actual} arguments; limit is {maximum}")]
    ArgumentLimit {
        operator: Identifier,
        actual: usize,
        maximum: usize,
    },
    #[error("operator {operator} version {version} is absent from the immutable registry")]
    UnknownOperator {
        operator: Identifier,
        version: PositiveInteger,
    },
    #[error("field {field} is absent from the immutable registry")]
    UnknownField { field: Identifier },
    #[error("enum value {enum_type}.{value} is absent from the immutable registry")]
    UnknownEnum {
        enum_type: Identifier,
        value: Identifier,
    },
    #[error("operator {operator} expects {minimum}..={maximum} arguments, found {actual}")]
    ArityMismatch {
        operator: Identifier,
        minimum: usize,
        maximum: usize,
        actual: usize,
    },
    #[error("operator {operator} argument {index} expects {expected}, found {actual}")]
    TypeMismatch {
        operator: Identifier,
        index: usize,
        expected: String,
        actual: String,
    },
    #[error("operator {operator} argument {index} must be a literal")]
    LiteralRequired { operator: Identifier, index: usize },
    #[error("decimal {value} violates its operator signature: {reason}")]
    DecimalConstraint {
        value: CanonicalDecimal,
        reason: String,
    },
    #[error("canonical AST has {actual} bytes; limit is {maximum}")]
    CanonicalByteLimit { actual: usize, maximum: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CanonicalizationError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("canonical AST JSON is invalid: {0}")]
    Parse(String),
    #[error("AST bytes differ from the dedicated canonical v1 writer output")]
    NonCanonical,
    #[error("factor expression root must resolve to series, found {actual}")]
    NonSeriesFactorRoot { actual: String },
    #[error("factor specification registry {claimed} does not match resolved registry {resolved}")]
    OperatorRegistryMismatch { claimed: String, resolved: String },
}

macro_rules! sha256_identity {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            pub fn parse(value: &str) -> Result<Self, IdentityParseError> {
                parse_digest(value).map(Self)
            }

            pub fn to_external(self) -> String {
                format!("sha256:{}", encode_digest(&self.0))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("sha256:")?;
                formatter.write_str(&encode_digest(&self.0))
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(D::Error::custom)
            }
        }
    };
}

sha256_identity!(ExpressionId);
sha256_identity!(FactorSpecId);
sha256_identity!(OperatorRegistryId);
sha256_identity!(SemanticContractId);

impl ExpressionId {
    pub fn verify(
        self,
        expression: &FactorExpr,
        registry: &OperatorPolicyRegistry,
        limits: ValidationLimits,
    ) -> Result<(), IdentityVerificationError> {
        let computed = expression_id(expression, registry, limits)?;
        if constant_time_digest_eq(self.as_bytes(), computed.as_bytes()) {
            Ok(())
        } else {
            Err(IdentityVerificationError::ExpressionMismatch {
                claimed: self,
                computed,
            })
        }
    }
}

impl FactorSpecId {
    pub fn verify(
        self,
        canonical_spec: &[u8],
        canonical_expression: &[u8],
        registry: &OperatorPolicyRegistry,
        limits: ValidationLimits,
    ) -> Result<FactorSpec, IdentityVerificationError> {
        parse_canonical_factor_spec(canonical_spec, self, canonical_expression, registry, limits)
    }
}

fn constant_time_digest_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn parse_digest(value: &str) -> Result<[u8; 32], IdentityParseError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(IdentityParseError::MissingPrefix);
    };
    if hex.len() != 64 {
        return Err(IdentityParseError::Length { actual: hex.len() });
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (decode_hex_nibble(pair[0], index * 2)? << 4)
            | decode_hex_nibble(pair[1], index * 2 + 1)?;
    }
    Ok(digest)
}

fn decode_hex_nibble(byte: u8, index: usize) -> Result<u8, IdentityParseError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(IdentityParseError::NonLowerHex { index, byte }),
    }
}

fn encode_digest(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentityParseError {
    #[error("SHA-256 identity must start with sha256:")]
    MissingPrefix,
    #[error("SHA-256 identity must contain exactly 64 lowercase hexadecimal bytes, found {actual}")]
    Length { actual: usize },
    #[error("SHA-256 identity contains non-lowercase-hex byte 0x{byte:02x} at offset {index}")]
    NonLowerHex { index: usize, byte: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentityVerificationError {
    #[error(transparent)]
    Canonicalization(#[from] CanonicalizationError),
    #[error("claimed expression ID {claimed} does not match computed ID {computed}")]
    ExpressionMismatch {
        claimed: ExpressionId,
        computed: ExpressionId,
    },
    #[error("claimed factor-spec ID {claimed} does not match computed ID {computed}")]
    FactorSpecMismatch {
        claimed: FactorSpecId,
        computed: FactorSpecId,
    },
}

#[cfg(test)]
mod tests;
