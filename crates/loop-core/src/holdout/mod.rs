//! Canonical identities for locked historical holdout periods and plans.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const PERIOD_SCHEMA: &str = "loop.holdout-period/v1";
const PLAN_SCHEMA: &str = "loop.holdout-evaluation-plan/v1";
const PERIOD_DOMAIN: &[u8] = b"loop.holdout-period/v1\0";
const PLAN_DOMAIN: &[u8] = b"loop.holdout-evaluation-plan/v1\0";
const BACKTEST_SCHEMA_NAME: &str = "loop.backtest_spec";
const PLAN_ARTIFACT_SCHEMA_NAME: &str = "loop.holdout_evaluation_plan";
const JSON_MEDIA_TYPE: &str = "application/json";

pub const MAX_PERIOD_BYTES: usize = 64 * 1_024;
pub const MAX_PLAN_BYTES: usize = 8 * 1_024 * 1_024;
pub const MAX_SNAPSHOTS: usize = 128;
pub const MAX_PLAN_ENTRIES: usize = 4_096;
pub const MAX_BACKTEST_ARTIFACT_BYTES: u64 = 268_435_456;
pub const MAXIMUM_STEPS: u64 = 1_000_000;
pub const MAXIMUM_TOKENS: u64 = 1_000_000_000_000;
pub const MAXIMUM_WALL_TIME_NS: u64 = 604_800_000_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoldoutValidationCode {
    InvalidJson,
    NonCanonical,
    SizeLimit,
    InvalidSchema,
    InvalidRole,
    InvalidDate,
    InvalidWindow,
    InvalidDigest,
    InvalidSnapshots,
    PeriodMismatch,
    InvalidEntries,
    DuplicateIdentity,
    InvalidArtifact,
    SchemaMismatch,
    InvalidBudget,
    UnresolvedArtifact,
    ReferenceMismatch,
}

impl HoldoutValidationCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid_json",
            Self::NonCanonical => "non_canonical",
            Self::SizeLimit => "size_limit",
            Self::InvalidSchema => "invalid_schema",
            Self::InvalidRole => "invalid_role",
            Self::InvalidDate => "invalid_date",
            Self::InvalidWindow => "invalid_window",
            Self::InvalidDigest => "invalid_digest",
            Self::InvalidSnapshots => "invalid_snapshots",
            Self::PeriodMismatch => "period_mismatch",
            Self::InvalidEntries => "invalid_entries",
            Self::DuplicateIdentity => "duplicate_identity",
            Self::InvalidArtifact => "invalid_artifact",
            Self::SchemaMismatch => "schema_mismatch",
            Self::InvalidBudget => "invalid_budget",
            Self::UnresolvedArtifact => "unresolved_artifact",
            Self::ReferenceMismatch => "reference_mismatch",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutValidationError {
    pub code: HoldoutValidationCode,
    pub field: &'static str,
}

impl HoldoutValidationError {
    const fn new(code: HoldoutValidationCode, field: &'static str) -> Self {
        Self { code, field }
    }
}

impl Display for HoldoutValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} failed holdout validation ({})",
            self.field,
            self.code.as_str()
        )
    }
}

impl Error for HoldoutValidationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockedSampleRole {
    FirstLockedConfirmation,
    SecondLockedHistoricalHoldout,
}

impl LockedSampleRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstLockedConfirmation => "first_locked_confirmation",
            Self::SecondLockedHistoricalHoldout => "second_locked_historical_holdout",
        }
    }

    fn parse(value: &str) -> Result<Self, HoldoutValidationError> {
        match value {
            "first_locked_confirmation" => Ok(Self::FirstLockedConfirmation),
            "second_locked_historical_holdout" => Ok(Self::SecondLockedHistoricalHoldout),
            _ => Err(HoldoutValidationError::new(
                HoldoutValidationCode::InvalidRole,
                "sample.role",
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutSampleWindow {
    pub role: LockedSampleRole,
    pub start_inclusive: String,
    pub end_inclusive: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutPeriod {
    pub sample: HoldoutSampleWindow,
    pub snapshot_ids: Vec<String>,
    pub snapshot_manifest_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalHoldoutPeriod {
    pub value: HoldoutPeriod,
    pub canonical_bytes: Vec<u8>,
    pub canonical_period_sha256: [u8; 32],
    pub holdout_period_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BacktestSpecArtifact {
    pub artifact_id: String,
    pub uri: String,
    pub sha256: String,
    pub schema_name: String,
    pub schema_version: String,
    pub schema_sha256: String,
    pub media_type: String,
    pub byte_size: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoneyBudget {
    pub amount: String,
    pub currency_code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutJobBudget {
    pub maximum_steps: String,
    pub maximum_input_tokens: String,
    pub maximum_output_tokens: String,
    pub maximum_cost: MoneyBudget,
    pub maximum_wall_time_ns: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutEvaluationPlanEntry {
    pub entry_index: String,
    pub factor_spec_id: String,
    pub backtest_spec_artifact: BacktestSpecArtifact,
    pub job_budget: HoldoutJobBudget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutEvaluationPlan {
    pub holdout_period_id: String,
    pub canonical_period_sha256: String,
    pub entries: Vec<HoldoutEvaluationPlanEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalHoldoutEvaluationPlan {
    pub value: HoldoutEvaluationPlan,
    pub canonical_bytes: Vec<u8>,
    pub plan_sha256: [u8; 32],
    pub holdout_evaluation_plan_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanArtifactReference {
    pub artifact_id: String,
    pub uri: String,
    pub sha256: [u8; 32],
    pub schema_name: String,
    pub schema_version: u32,
    pub schema_sha256: [u8; 32],
    pub media_type: String,
    pub byte_size: u64,
    pub has_row_count: bool,
    pub has_manifest_sha256: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutEvaluationPlanReference {
    pub holdout_evaluation_plan_id: String,
    pub canonical_plan: PlanArtifactReference,
    pub plan_sha256: [u8; 32],
    pub entry_count: u32,
    pub holdout_period_id: String,
    pub canonical_period_sha256: [u8; 32],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPeriod {
    schema: String,
    sample: RawSample,
    snapshot_ids: Vec<String>,
    snapshot_manifest_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSample {
    role: String,
    start_inclusive: String,
    end_inclusive: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlan {
    schema: String,
    holdout_period_id: String,
    canonical_period_sha256: String,
    entries: Vec<RawPlanEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlanEntry {
    entry_index: String,
    factor_spec_id: String,
    backtest_spec_artifact: RawBacktestArtifact,
    job_budget: RawBudget,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBacktestArtifact {
    artifact_id: String,
    uri: String,
    sha256: String,
    schema_name: String,
    schema_version: String,
    schema_sha256: String,
    media_type: String,
    byte_size: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudget {
    maximum_steps: String,
    maximum_input_tokens: String,
    maximum_output_tokens: String,
    maximum_cost: RawMoney,
    maximum_wall_time_ns: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMoney {
    amount: String,
    currency_code: String,
}

pub fn canonicalize_holdout_period(
    value: HoldoutPeriod,
) -> Result<CanonicalHoldoutPeriod, HoldoutValidationError> {
    validate_period(&value)?;
    let mut canonical_bytes = Vec::new();
    write_period(&mut canonical_bytes, &value);
    if canonical_bytes.len() > MAX_PERIOD_BYTES {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::SizeLimit,
            "period",
        ));
    }
    let digest = domain_hash(PERIOD_DOMAIN, &canonical_bytes);
    Ok(CanonicalHoldoutPeriod {
        value,
        canonical_bytes,
        canonical_period_sha256: digest,
        holdout_period_id: encode_digest(&digest),
    })
}

pub fn parse_canonical_holdout_period(
    canonical: &[u8],
) -> Result<CanonicalHoldoutPeriod, HoldoutValidationError> {
    validate_json_envelope(canonical, MAX_PERIOD_BYTES)?;
    let raw: RawPeriod = serde_json::from_slice(canonical)
        .map_err(|_| HoldoutValidationError::new(HoldoutValidationCode::InvalidJson, "period"))?;
    if raw.schema != PERIOD_SCHEMA {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidSchema,
            "schema",
        ));
    }
    let value = HoldoutPeriod {
        sample: HoldoutSampleWindow {
            role: LockedSampleRole::parse(&raw.sample.role)?,
            start_inclusive: raw.sample.start_inclusive,
            end_inclusive: raw.sample.end_inclusive,
        },
        snapshot_ids: raw.snapshot_ids,
        snapshot_manifest_sha256: raw.snapshot_manifest_sha256,
    };
    let parsed = canonicalize_holdout_period(value)?;
    if !constant_time_bytes_eq(&parsed.canonical_bytes, canonical) {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::NonCanonical,
            "period",
        ));
    }
    Ok(parsed)
}

pub fn verify_holdout_period_identity(
    canonical: &[u8],
    holdout_period_id: &str,
    canonical_period_sha256: &[u8; 32],
) -> Result<CanonicalHoldoutPeriod, HoldoutValidationError> {
    let period = parse_canonical_holdout_period(canonical)?;
    if period.holdout_period_id != holdout_period_id
        || !constant_time_digest_eq(&period.canonical_period_sha256, canonical_period_sha256)
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::PeriodMismatch,
            "holdout_period_id",
        ));
    }
    Ok(period)
}

pub fn canonicalize_holdout_evaluation_plan(
    value: HoldoutEvaluationPlan,
    expected_period: &CanonicalHoldoutPeriod,
    trusted_backtest_schema_sha256: &[u8; 32],
    resolved_backtest_artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<CanonicalHoldoutEvaluationPlan, HoldoutValidationError> {
    validate_plan(
        &value,
        expected_period,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )?;
    let mut canonical_bytes = Vec::new();
    write_plan(&mut canonical_bytes, &value);
    if canonical_bytes.len() > MAX_PLAN_BYTES {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::SizeLimit,
            "plan",
        ));
    }
    let plan_sha256 = raw_hash(&canonical_bytes);
    let plan_id_digest = domain_hash(PLAN_DOMAIN, &canonical_bytes);
    Ok(CanonicalHoldoutEvaluationPlan {
        value,
        canonical_bytes,
        plan_sha256,
        holdout_evaluation_plan_id: encode_digest(&plan_id_digest),
    })
}

pub fn parse_canonical_holdout_evaluation_plan(
    canonical: &[u8],
    expected_period: &CanonicalHoldoutPeriod,
    trusted_backtest_schema_sha256: &[u8; 32],
    resolved_backtest_artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<CanonicalHoldoutEvaluationPlan, HoldoutValidationError> {
    validate_json_envelope(canonical, MAX_PLAN_BYTES)?;
    let raw: RawPlan = serde_json::from_slice(canonical)
        .map_err(|_| HoldoutValidationError::new(HoldoutValidationCode::InvalidJson, "plan"))?;
    if raw.schema != PLAN_SCHEMA {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidSchema,
            "schema",
        ));
    }
    let value = HoldoutEvaluationPlan {
        holdout_period_id: raw.holdout_period_id,
        canonical_period_sha256: raw.canonical_period_sha256,
        entries: raw
            .entries
            .into_iter()
            .map(|entry| HoldoutEvaluationPlanEntry {
                entry_index: entry.entry_index,
                factor_spec_id: entry.factor_spec_id,
                backtest_spec_artifact: BacktestSpecArtifact {
                    artifact_id: entry.backtest_spec_artifact.artifact_id,
                    uri: entry.backtest_spec_artifact.uri,
                    sha256: entry.backtest_spec_artifact.sha256,
                    schema_name: entry.backtest_spec_artifact.schema_name,
                    schema_version: entry.backtest_spec_artifact.schema_version,
                    schema_sha256: entry.backtest_spec_artifact.schema_sha256,
                    media_type: entry.backtest_spec_artifact.media_type,
                    byte_size: entry.backtest_spec_artifact.byte_size,
                },
                job_budget: HoldoutJobBudget {
                    maximum_steps: entry.job_budget.maximum_steps,
                    maximum_input_tokens: entry.job_budget.maximum_input_tokens,
                    maximum_output_tokens: entry.job_budget.maximum_output_tokens,
                    maximum_cost: MoneyBudget {
                        amount: entry.job_budget.maximum_cost.amount,
                        currency_code: entry.job_budget.maximum_cost.currency_code,
                    },
                    maximum_wall_time_ns: entry.job_budget.maximum_wall_time_ns,
                },
            })
            .collect(),
    };
    let parsed = canonicalize_holdout_evaluation_plan(
        value,
        expected_period,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )?;
    if !constant_time_bytes_eq(&parsed.canonical_bytes, canonical) {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::NonCanonical,
            "plan",
        ));
    }
    Ok(parsed)
}

pub fn validate_holdout_evaluation_plan_reference(
    reference: &HoldoutEvaluationPlanReference,
    canonical_plan_bytes: &[u8],
    expected_period: &CanonicalHoldoutPeriod,
    trusted_plan_schema_sha256: &[u8; 32],
    trusted_backtest_schema_sha256: &[u8; 32],
    resolved_backtest_artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<CanonicalHoldoutEvaluationPlan, HoldoutValidationError> {
    let parsed = parse_canonical_holdout_evaluation_plan(
        canonical_plan_bytes,
        expected_period,
        trusted_backtest_schema_sha256,
        resolved_backtest_artifacts,
    )?;
    let artifact = &reference.canonical_plan;
    let raw_id = encode_digest(&parsed.plan_sha256);
    let expected_uri = format!("artifact://sha256/{}", &raw_id[7..]);
    if artifact.artifact_id != raw_id
        || artifact.uri != expected_uri
        || !constant_time_digest_eq(&artifact.sha256, &parsed.plan_sha256)
        || !constant_time_digest_eq(&reference.plan_sha256, &parsed.plan_sha256)
        || artifact.byte_size != canonical_plan_bytes.len() as u64
        || artifact.has_row_count
        || artifact.has_manifest_sha256
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::ReferenceMismatch,
            "canonical_plan",
        ));
    }
    if artifact.schema_name != PLAN_ARTIFACT_SCHEMA_NAME
        || artifact.schema_version != 1
        || artifact.media_type != JSON_MEDIA_TYPE
        || !constant_time_digest_eq(&artifact.schema_sha256, trusted_plan_schema_sha256)
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::SchemaMismatch,
            "canonical_plan.schema",
        ));
    }
    if reference.holdout_evaluation_plan_id != parsed.holdout_evaluation_plan_id
        || usize::try_from(reference.entry_count).ok() != Some(parsed.value.entries.len())
        || reference.holdout_period_id != expected_period.holdout_period_id
        || !constant_time_digest_eq(
            &reference.canonical_period_sha256,
            &expected_period.canonical_period_sha256,
        )
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::ReferenceMismatch,
            "plan_reference",
        ));
    }
    Ok(parsed)
}

fn validate_period(value: &HoldoutPeriod) -> Result<(), HoldoutValidationError> {
    let start = parse_date(&value.sample.start_inclusive)?;
    let end = parse_date(&value.sample.end_inclusive)?;
    if start > end {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidWindow,
            "sample",
        ));
    }
    if value.snapshot_ids.is_empty() || value.snapshot_ids.len() > MAX_SNAPSHOTS {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidSnapshots,
            "snapshot_ids",
        ));
    }
    let mut previous: Option<&str> = None;
    for snapshot_id in &value.snapshot_ids {
        require_digest_text(snapshot_id, "snapshot_ids")?;
        if previous.is_some_and(|prior| prior >= snapshot_id.as_str()) {
            return Err(HoldoutValidationError::new(
                HoldoutValidationCode::InvalidSnapshots,
                "snapshot_ids",
            ));
        }
        previous = Some(snapshot_id);
    }
    require_digest_text(&value.snapshot_manifest_sha256, "snapshot_manifest_sha256")?;
    Ok(())
}

fn validate_plan(
    value: &HoldoutEvaluationPlan,
    expected_period: &CanonicalHoldoutPeriod,
    trusted_backtest_schema_sha256: &[u8; 32],
    resolved: &BTreeMap<String, Vec<u8>>,
) -> Result<(), HoldoutValidationError> {
    let expected_period_id = &expected_period.holdout_period_id;
    if &value.holdout_period_id != expected_period_id
        || &value.canonical_period_sha256 != expected_period_id
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::PeriodMismatch,
            "holdout_period_id",
        ));
    }
    if value.entries.is_empty() || value.entries.len() > MAX_PLAN_ENTRIES {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidEntries,
            "entries",
        ));
    }
    let mut factors = BTreeSet::new();
    let mut artifacts = BTreeSet::new();
    for (position, entry) in value.entries.iter().enumerate() {
        let expected_index = position + 1;
        if parse_unsigned(
            &entry.entry_index,
            1,
            MAX_PLAN_ENTRIES as u64,
            "entry_index",
        )? != expected_index as u64
        {
            return Err(HoldoutValidationError::new(
                HoldoutValidationCode::InvalidEntries,
                "entry_index",
            ));
        }
        require_digest_text(&entry.factor_spec_id, "factor_spec_id")?;
        if !factors.insert(entry.factor_spec_id.as_str()) {
            return Err(HoldoutValidationError::new(
                HoldoutValidationCode::DuplicateIdentity,
                "factor_spec_id",
            ));
        }
        validate_backtest_artifact(
            &entry.backtest_spec_artifact,
            trusted_backtest_schema_sha256,
            resolved,
        )?;
        if !artifacts.insert(entry.backtest_spec_artifact.artifact_id.as_str()) {
            return Err(HoldoutValidationError::new(
                HoldoutValidationCode::DuplicateIdentity,
                "backtest_spec_artifact.artifact_id",
            ));
        }
        validate_budget(&entry.job_budget)?;
    }
    Ok(())
}

fn validate_backtest_artifact(
    artifact: &BacktestSpecArtifact,
    trusted_schema_sha256: &[u8; 32],
    resolved: &BTreeMap<String, Vec<u8>>,
) -> Result<(), HoldoutValidationError> {
    let digest = require_digest_text(&artifact.sha256, "backtest_spec_artifact.sha256")?;
    if artifact.artifact_id != artifact.sha256
        || artifact.uri != format!("artifact://sha256/{}", &artifact.sha256[7..])
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidArtifact,
            "backtest_spec_artifact",
        ));
    }
    if artifact.schema_name != BACKTEST_SCHEMA_NAME
        || artifact.schema_version != "1"
        || artifact.media_type != JSON_MEDIA_TYPE
        || require_digest_text(
            &artifact.schema_sha256,
            "backtest_spec_artifact.schema_sha256",
        )? != *trusted_schema_sha256
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::SchemaMismatch,
            "backtest_spec_artifact.schema",
        ));
    }
    let declared_size = parse_unsigned(
        &artifact.byte_size,
        1,
        MAX_BACKTEST_ARTIFACT_BYTES,
        "backtest_spec_artifact.byte_size",
    )?;
    let content = resolved.get(&artifact.sha256).ok_or_else(|| {
        HoldoutValidationError::new(
            HoldoutValidationCode::UnresolvedArtifact,
            "backtest_spec_artifact",
        )
    })?;
    if content.len() as u64 != declared_size
        || !constant_time_digest_eq(&raw_hash(content), &digest)
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidArtifact,
            "backtest_spec_artifact",
        ));
    }
    Ok(())
}

fn validate_budget(value: &HoldoutJobBudget) -> Result<(), HoldoutValidationError> {
    parse_unsigned(&value.maximum_steps, 1, MAXIMUM_STEPS, "maximum_steps")?;
    parse_unsigned(
        &value.maximum_input_tokens,
        0,
        MAXIMUM_TOKENS,
        "maximum_input_tokens",
    )?;
    parse_unsigned(
        &value.maximum_output_tokens,
        0,
        MAXIMUM_TOKENS,
        "maximum_output_tokens",
    )?;
    parse_unsigned(
        &value.maximum_wall_time_ns,
        1,
        MAXIMUM_WALL_TIME_NS,
        "maximum_wall_time_ns",
    )?;
    validate_cost(&value.maximum_cost)
}

fn validate_cost(value: &MoneyBudget) -> Result<(), HoldoutValidationError> {
    if value.currency_code.len() != 3
        || !value
            .currency_code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidBudget,
            "maximum_cost.currency_code",
        ));
    }
    let amount = value.amount.as_str();
    if amount.is_empty() || amount.starts_with('-') || amount.starts_with('+') {
        return invalid_budget("maximum_cost.amount");
    }
    let mut parts = amount.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || (integer.len() > 1 && integer.starts_with('0'))
        || fraction.is_some_and(|part| {
            part.is_empty()
                || part.len() > 9
                || part.ends_with('0')
                || !part.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return invalid_budget("maximum_cost.amount");
    }
    let significant = if integer == "0" {
        fraction.map_or(1, |part| part.trim_start_matches('0').len().max(1))
    } else {
        integer.len() + fraction.map_or(0, str::len)
    };
    if significant > 18
        || integer.len() > 7
        || (integer.len() == 7
            && (integer > "1000000" || (integer == "1000000" && fraction.is_some())))
    {
        return invalid_budget("maximum_cost.amount");
    }
    Ok(())
}

fn invalid_budget<T>(field: &'static str) -> Result<T, HoldoutValidationError> {
    Err(HoldoutValidationError::new(
        HoldoutValidationCode::InvalidBudget,
        field,
    ))
}

fn parse_unsigned(
    value: &str,
    minimum: u64,
    maximum: u64,
    field: &'static str,
) -> Result<u64, HoldoutValidationError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return invalid_budget(field);
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| HoldoutValidationError::new(HoldoutValidationCode::InvalidBudget, field))?;
    if !(minimum..=maximum).contains(&parsed) {
        return invalid_budget(field);
    }
    Ok(parsed)
}

fn parse_date(value: &str) -> Result<(u16, u8, u8), HoldoutValidationError> {
    if value.len() != 10
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return invalid_date();
    }
    let year = value[0..4].parse::<u16>().map_err(|_| date_error())?;
    let month = value[5..7].parse::<u8>().map_err(|_| date_error())?;
    let day = value[8..10].parse::<u8>().map_err(|_| date_error())?;
    if !(1900..=9999).contains(&year) || !(1..=12).contains(&month) {
        return invalid_date();
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let maximum_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > maximum_day {
        return invalid_date();
    }
    Ok((year, month, day))
}

const fn date_error() -> HoldoutValidationError {
    HoldoutValidationError::new(HoldoutValidationCode::InvalidDate, "sample.date")
}

fn invalid_date<T>() -> Result<T, HoldoutValidationError> {
    Err(date_error())
}

fn require_digest_text(
    value: &str,
    field: &'static str,
) -> Result<[u8; 32], HoldoutValidationError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidDigest,
            field,
        ));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::InvalidDigest,
            field,
        ));
    }
    let mut digest = [0_u8; 32];
    for (index, output) in digest.iter_mut().enumerate() {
        *output =
            (hex_value(hex.as_bytes()[index * 2]) << 4) | hex_value(hex.as_bytes()[index * 2 + 1]);
    }
    Ok(digest)
}

const fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

fn validate_json_envelope(
    canonical: &[u8],
    maximum_size: usize,
) -> Result<(), HoldoutValidationError> {
    if canonical.is_empty() || canonical.len() > maximum_size {
        return Err(HoldoutValidationError::new(
            HoldoutValidationCode::SizeLimit,
            "json",
        ));
    }
    let mut depth = 0_u8;
    let mut in_string = false;
    let mut escaped = false;
    for byte in canonical {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.checked_add(1).ok_or_else(|| {
                    HoldoutValidationError::new(HoldoutValidationCode::SizeLimit, "json.depth")
                })?;
                if depth > 32 {
                    return Err(HoldoutValidationError::new(
                        HoldoutValidationCode::SizeLimit,
                        "json.depth",
                    ));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

fn write_period(output: &mut Vec<u8>, value: &HoldoutPeriod) {
    output.extend_from_slice(b"{\"schema\":\"loop.holdout-period/v1\",\"sample\":{\"role\":\"");
    output.extend_from_slice(value.sample.role.as_str().as_bytes());
    output.extend_from_slice(b"\",\"start_inclusive\":\"");
    output.extend_from_slice(value.sample.start_inclusive.as_bytes());
    output.extend_from_slice(b"\",\"end_inclusive\":\"");
    output.extend_from_slice(value.sample.end_inclusive.as_bytes());
    output.extend_from_slice(b"\"},\"snapshot_ids\":[");
    for (index, snapshot_id) in value.snapshot_ids.iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        write_string(output, snapshot_id);
    }
    output.extend_from_slice(b"],\"snapshot_manifest_sha256\":");
    write_string(output, &value.snapshot_manifest_sha256);
    output.push(b'}');
}

fn write_plan(output: &mut Vec<u8>, value: &HoldoutEvaluationPlan) {
    output.extend_from_slice(
        b"{\"schema\":\"loop.holdout-evaluation-plan/v1\",\"holdout_period_id\":",
    );
    write_string(output, &value.holdout_period_id);
    output.extend_from_slice(b",\"canonical_period_sha256\":");
    write_string(output, &value.canonical_period_sha256);
    output.extend_from_slice(b",\"entries\":[");
    for (index, entry) in value.entries.iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        output.extend_from_slice(b"{\"entry_index\":");
        write_string(output, &entry.entry_index);
        output.extend_from_slice(b",\"factor_spec_id\":");
        write_string(output, &entry.factor_spec_id);
        output.extend_from_slice(b",\"backtest_spec_artifact\":{");
        write_named_string(
            output,
            "artifact_id",
            &entry.backtest_spec_artifact.artifact_id,
            false,
        );
        write_named_string(output, "uri", &entry.backtest_spec_artifact.uri, true);
        write_named_string(output, "sha256", &entry.backtest_spec_artifact.sha256, true);
        write_named_string(
            output,
            "schema_name",
            &entry.backtest_spec_artifact.schema_name,
            true,
        );
        write_named_string(
            output,
            "schema_version",
            &entry.backtest_spec_artifact.schema_version,
            true,
        );
        write_named_string(
            output,
            "schema_sha256",
            &entry.backtest_spec_artifact.schema_sha256,
            true,
        );
        write_named_string(
            output,
            "media_type",
            &entry.backtest_spec_artifact.media_type,
            true,
        );
        write_named_string(
            output,
            "byte_size",
            &entry.backtest_spec_artifact.byte_size,
            true,
        );
        output.extend_from_slice(b"},\"job_budget\":{");
        write_named_string(
            output,
            "maximum_steps",
            &entry.job_budget.maximum_steps,
            false,
        );
        write_named_string(
            output,
            "maximum_input_tokens",
            &entry.job_budget.maximum_input_tokens,
            true,
        );
        write_named_string(
            output,
            "maximum_output_tokens",
            &entry.job_budget.maximum_output_tokens,
            true,
        );
        output.extend_from_slice(b",\"maximum_cost\":{");
        write_named_string(
            output,
            "amount",
            &entry.job_budget.maximum_cost.amount,
            false,
        );
        write_named_string(
            output,
            "currency_code",
            &entry.job_budget.maximum_cost.currency_code,
            true,
        );
        output.extend_from_slice(b"},\"maximum_wall_time_ns\":");
        write_string(output, &entry.job_budget.maximum_wall_time_ns);
        output.extend_from_slice(b"}}");
    }
    output.extend_from_slice(b"]}");
}

fn write_named_string(output: &mut Vec<u8>, name: &str, value: &str, comma: bool) {
    if comma {
        output.push(b',');
    }
    output.push(b'"');
    output.extend_from_slice(name.as_bytes());
    output.extend_from_slice(b"\":");
    write_string(output, value);
}

fn write_string(output: &mut Vec<u8>, value: &str) {
    output.push(b'"');
    output.extend_from_slice(value.as_bytes());
    output.push(b'"');
}

fn raw_hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().into()
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

fn constant_time_digest_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn constant_time_bytes_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests;
