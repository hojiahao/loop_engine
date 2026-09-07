use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    Identifier, OperatorRef, PositiveInteger, RegistryError, SemanticContractId,
    constant_time_digest_eq,
};

pub const OPERATOR_SEMANTIC_CONTRACT_SCHEMA: &str = "loop.operator-semantic-contract/v1";
const MAX_SEMANTIC_CONTRACT_BYTES: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullPolicy {
    NotApplicable,
    Propagate,
    IgnoreMissing,
    PreserveTargetIgnorePeers,
    RejectMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowPolicy {
    NotApplicable,
    #[serde(rename = "trailing_argument_2_full_window_right_inclusive_constant_preserve")]
    TrailingArgument2FullWindowRightInclusiveConstantPreserve,
    #[serde(
        rename = "trailing_argument_2_minimum_valid_min_n_max_3_floor_2n_div_3_right_inclusive_constant_preserve"
    )]
    TrailingArgument2MinimumValidMinNMax3Floor2NDiv3RightInclusiveConstantPreserve,
    #[serde(rename = "lag_argument_2")]
    LagArgument2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TiePolicy {
    NotApplicable,
    AverageValidCount,
    DenseValidCount,
    StableFirstValidCountMinusOne,
    #[serde(rename = "argument_2_average_or_dense_valid_count_constant_midpoint")]
    Argument2AverageOrDenseValidCountConstantMidpoint,
    TargetLastStableOrderValidCountMinusOneConstantMidpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentPolicy {
    NotApplicable,
    UnaryPreserveTimestampAndSecurity,
    StrictTimestampAndSecurity,
    IntersectionTimestampAndSecurity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumericPolicy {
    NotApplicable,
    ExactDecimal,
    #[serde(rename = "binary64_non_finite_to_missing")]
    Binary64NonFiniteToMissing,
    #[serde(rename = "binary64_reject_non_finite")]
    Binary64RejectNonFinite,
    OrdinalUnitInterval,
    #[serde(
        rename = "binary64_adjusted_fisher_pearson_effective_n_minimum_3_constant_zero_non_finite_to_missing"
    )]
    Binary64AdjustedFisherPearsonEffectiveNMinimum3ConstantZeroNonFiniteToMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OperatorSemanticContract {
    schema: String,
    operator: Identifier,
    operator_version: PositiveInteger,
    null_policy: NullPolicy,
    window_policy: WindowPolicy,
    tie_policy: TiePolicy,
    alignment_policy: AlignmentPolicy,
    numeric_policy: NumericPolicy,
}

impl OperatorSemanticContract {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operator: Identifier,
        operator_version: PositiveInteger,
        null_policy: NullPolicy,
        window_policy: WindowPolicy,
        tie_policy: TiePolicy,
        alignment_policy: AlignmentPolicy,
        numeric_policy: NumericPolicy,
    ) -> Self {
        Self {
            schema: OPERATOR_SEMANTIC_CONTRACT_SCHEMA.to_owned(),
            operator,
            operator_version,
            null_policy,
            window_policy,
            tie_policy,
            alignment_policy,
            numeric_policy,
        }
    }

    pub fn operator(&self) -> &Identifier {
        &self.operator
    }

    pub fn operator_version(&self) -> &PositiveInteger {
        &self.operator_version
    }

    pub fn null_policy(&self) -> NullPolicy {
        self.null_policy
    }

    pub fn window_policy(&self) -> WindowPolicy {
        self.window_policy
    }

    pub fn tie_policy(&self) -> TiePolicy {
        self.tie_policy
    }

    pub fn alignment_policy(&self) -> AlignmentPolicy {
        self.alignment_policy
    }

    pub fn numeric_policy(&self) -> NumericPolicy {
        self.numeric_policy
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();
        output.extend_from_slice(b"{\"schema\":\"");
        output.extend_from_slice(OPERATOR_SEMANTIC_CONTRACT_SCHEMA.as_bytes());
        output.extend_from_slice(b"\",\"operator\":\"");
        output.extend_from_slice(self.operator.as_str().as_bytes());
        output.extend_from_slice(b"\",\"operatorVersion\":\"");
        output.extend_from_slice(self.operator_version.as_str().as_bytes());
        output.extend_from_slice(b"\",\"nullPolicy\":\"");
        output.extend_from_slice(null_policy_name(self.null_policy).as_bytes());
        output.extend_from_slice(b"\",\"windowPolicy\":\"");
        output.extend_from_slice(window_policy_name(self.window_policy).as_bytes());
        output.extend_from_slice(b"\",\"tiePolicy\":\"");
        output.extend_from_slice(tie_policy_name(self.tie_policy).as_bytes());
        output.extend_from_slice(b"\",\"alignmentPolicy\":\"");
        output.extend_from_slice(alignment_policy_name(self.alignment_policy).as_bytes());
        output.extend_from_slice(b"\",\"numericPolicy\":\"");
        output.extend_from_slice(numeric_policy_name(self.numeric_policy).as_bytes());
        output.extend_from_slice(b"\"}");
        output
    }

    pub fn identity(&self) -> SemanticContractId {
        semantic_contract_sha256(&self.canonical_bytes())
    }
}

pub trait SemanticContractResolver {
    fn resolve(&self, identity: SemanticContractId) -> Option<Vec<u8>>;
}

impl SemanticContractResolver for BTreeMap<SemanticContractId, Vec<u8>> {
    fn resolve(&self, identity: SemanticContractId) -> Option<Vec<u8>> {
        self.get(&identity).cloned()
    }
}

impl<F> SemanticContractResolver for F
where
    F: Fn(SemanticContractId) -> Option<Vec<u8>>,
{
    fn resolve(&self, identity: SemanticContractId) -> Option<Vec<u8>> {
        self(identity)
    }
}

pub fn parse_canonical_operator_semantic_contract(
    bytes: &[u8],
) -> Result<OperatorSemanticContract, RegistryError> {
    if bytes.len() > MAX_SEMANTIC_CONTRACT_BYTES {
        return Err(RegistryError::InvalidSemanticContract(
            "semantic contract exceeds the v1 byte limit".to_owned(),
        ));
    }
    let contract: OperatorSemanticContract = serde_json::from_slice(bytes)
        .map_err(|error| RegistryError::InvalidSemanticContract(error.to_string()))?;
    if contract.schema != OPERATOR_SEMANTIC_CONTRACT_SCHEMA {
        return Err(RegistryError::InvalidSemanticContract(format!(
            "unsupported semantic contract schema {}",
            contract.schema
        )));
    }
    if contract.canonical_bytes() != bytes {
        return Err(RegistryError::NonCanonicalSemanticContract);
    }
    Ok(contract)
}

pub fn semantic_contract_sha256(bytes: &[u8]) -> SemanticContractId {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    SemanticContractId::from_bytes(hasher.finalize().into())
}

pub(super) fn resolve_semantic_contract(
    operator: &OperatorRef,
    identity: SemanticContractId,
    resolver: &impl SemanticContractResolver,
) -> Result<OperatorSemanticContract, RegistryError> {
    let bytes = resolver
        .resolve(identity)
        .ok_or(RegistryError::SemanticContractNotFound { identity })?;
    let computed = semantic_contract_sha256(&bytes);
    if !constant_time_digest_eq(identity.as_bytes(), computed.as_bytes()) {
        return Err(RegistryError::SemanticContractDigestMismatch {
            claimed: identity,
            computed,
        });
    }
    let contract = parse_canonical_operator_semantic_contract(&bytes)?;
    if contract.operator() != operator.name()
        || contract.operator_version() != operator.semantic_version()
    {
        return Err(RegistryError::SemanticContractOperatorMismatch {
            expected: operator.clone(),
            actual: OperatorRef::new(
                contract.operator().clone(),
                contract.operator_version().clone(),
            ),
        });
    }
    Ok(contract)
}

fn null_policy_name(value: NullPolicy) -> &'static str {
    match value {
        NullPolicy::NotApplicable => "not_applicable",
        NullPolicy::Propagate => "propagate",
        NullPolicy::IgnoreMissing => "ignore_missing",
        NullPolicy::PreserveTargetIgnorePeers => "preserve_target_ignore_peers",
        NullPolicy::RejectMissing => "reject_missing",
    }
}

fn window_policy_name(value: WindowPolicy) -> &'static str {
    match value {
        WindowPolicy::NotApplicable => "not_applicable",
        WindowPolicy::TrailingArgument2FullWindowRightInclusiveConstantPreserve => {
            "trailing_argument_2_full_window_right_inclusive_constant_preserve"
        }
        WindowPolicy::TrailingArgument2MinimumValidMinNMax3Floor2NDiv3RightInclusiveConstantPreserve => {
            "trailing_argument_2_minimum_valid_min_n_max_3_floor_2n_div_3_right_inclusive_constant_preserve"
        }
        WindowPolicy::LagArgument2 => "lag_argument_2",
    }
}

fn tie_policy_name(value: TiePolicy) -> &'static str {
    match value {
        TiePolicy::NotApplicable => "not_applicable",
        TiePolicy::AverageValidCount => "average_valid_count",
        TiePolicy::DenseValidCount => "dense_valid_count",
        TiePolicy::StableFirstValidCountMinusOne => "stable_first_valid_count_minus_one",
        TiePolicy::Argument2AverageOrDenseValidCountConstantMidpoint => {
            "argument_2_average_or_dense_valid_count_constant_midpoint"
        }
        TiePolicy::TargetLastStableOrderValidCountMinusOneConstantMidpoint => {
            "target_last_stable_order_valid_count_minus_one_constant_midpoint"
        }
    }
}

fn alignment_policy_name(value: AlignmentPolicy) -> &'static str {
    match value {
        AlignmentPolicy::NotApplicable => "not_applicable",
        AlignmentPolicy::UnaryPreserveTimestampAndSecurity => {
            "unary_preserve_timestamp_and_security"
        }
        AlignmentPolicy::StrictTimestampAndSecurity => "strict_timestamp_and_security",
        AlignmentPolicy::IntersectionTimestampAndSecurity => "intersection_timestamp_and_security",
    }
}

fn numeric_policy_name(value: NumericPolicy) -> &'static str {
    match value {
        NumericPolicy::NotApplicable => "not_applicable",
        NumericPolicy::ExactDecimal => "exact_decimal",
        NumericPolicy::Binary64NonFiniteToMissing => "binary64_non_finite_to_missing",
        NumericPolicy::Binary64RejectNonFinite => "binary64_reject_non_finite",
        NumericPolicy::OrdinalUnitInterval => "ordinal_unit_interval",
        NumericPolicy::Binary64AdjustedFisherPearsonEffectiveNMinimum3ConstantZeroNonFiniteToMissing => {
            "binary64_adjusted_fisher_pearson_effective_n_minimum_3_constant_zero_non_finite_to_missing"
        }
    }
}
