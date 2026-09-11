use chrono::NaiveDate;
use loop_core::factor::{CanonicalDecimal, Identifier, PolicyId, PositiveInteger};
use loop_protocol::artifact::validate_artifact_ref;
use loop_protocol::wire::v1::{
    ArtifactId, ArtifactRef, ArtifactSchemaReference, BacktestArtifacts, BacktestEngineKind,
    BacktestMetric, DevelopmentDatasetReference, ExactDecimal, ResearchProvenanceFingerprint,
    Sha256Digest, SnapshotId,
};
use serde::{Deserialize, Serialize};

use super::ObjectRef;
use crate::store::{StoreError, StoreResult};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    pub schema: String,
    pub contexts: Vec<ContextEntry>,
    pub backtests: Vec<BacktestEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContextEntry {
    pub context_id: String,
    pub manifest: ObjectRef,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BacktestEntry {
    pub job_id: String,
    pub specification: ObjectRef,
    pub result: Option<Artifact>,
    pub review: Option<Artifact>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Context {
    pub schema: String,
    pub source: ObjectRef,
    pub registry: ObjectRef,
    pub configuration: ObjectRef,
    pub data: ObjectRef,
    pub calendar: ObjectRef,
    pub environment: ObjectRef,
    pub family: Option<ObjectRef>,
}

impl Context {
    pub fn provenance(&self, registry_id: [u8; 32]) -> StoreResult<ResearchProvenanceFingerprint> {
        Ok(ResearchProvenanceFingerprint {
            source_code_sha256: Some(digest(self.source.digest()?)),
            operator_registry_sha256: Some(digest(registry_id)),
            configuration_sha256: Some(digest(self.configuration.digest()?)),
            data_manifest_sha256: Some(digest(self.data.digest()?)),
            trading_calendar_sha256: Some(digest(self.calendar.digest()?)),
            environment_sha256: Some(digest(self.environment.digest()?)),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileSet {
    pub schema: String,
    pub files: Vec<NamedFile>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NamedFile {
    pub name: String,
    pub object: ObjectRef,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    pub schema: String,
    pub backtest_engine: Engine,
    pub backtest_engine_version: String,
    pub policies: Vec<Policy>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    pub policy_id: String,
    pub revision: String,
    pub document: ObjectRef,
}

impl Policy {
    pub fn validate(&self) -> StoreResult<()> {
        PolicyId::new(&self.policy_id).map_err(|_| StoreError::Corrupt("policy ID"))?;
        PositiveInteger::new(&self.revision).map_err(|_| StoreError::Corrupt("policy revision"))?;
        self.document.digest()?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicyDocument {
    pub schema: String,
    pub policy_id: String,
    pub revision: String,
    pub settings: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Dataset {
    pub schema: String,
    pub sample: Sample,
    pub quality: Quality,
    pub snapshots: Vec<Snapshot>,
}

impl Dataset {
    pub fn reference(&self, manifest: &ObjectRef) -> StoreResult<DevelopmentDatasetReference> {
        Ok(DevelopmentDatasetReference {
            snapshot_ids: self
                .snapshots
                .iter()
                .map(|snapshot| SnapshotId {
                    value: snapshot.snapshot_id.clone(),
                })
                .collect(),
            manifest_sha256: Some(digest(manifest.digest()?)),
        })
    }
}

// Licensed/PIT production claims require Phase 5's dedicated data-quality gate.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Quality {
    Synthetic,
    PublicDevelopment,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    pub snapshot_id: String,
    pub source: String,
    pub dataset: String,
    pub entitlement: String,
    pub known_through_ms: i64,
    pub artifacts: Vec<Artifact>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Sample {
    pub role: SampleRole,
    pub start: String,
    pub end: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SampleRole {
    OperatorWarmup,
    InSample,
    DevelopmentValidation,
}

impl Sample {
    pub fn validate(&self) -> StoreResult<()> {
        let start = date(&self.start)?;
        let end = date(&self.end)?;
        let (lower, upper) = match self.role {
            SampleRole::OperatorWarmup => ("2005-01-01", "2006-12-31"),
            SampleRole::InSample => ("2007-01-01", "2016-12-31"),
            SampleRole::DevelopmentValidation => ("2017-01-01", "2020-12-31"),
        };
        if start > end || start < date(lower)? || end > date(upper)? {
            return Err(StoreError::AdmissionDenied);
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Calendar {
    pub schema: String,
    pub name: String,
    pub timezone: String,
    pub sessions: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Factor {
    pub schema: String,
    pub factor_spec_id: String,
    pub specification: ObjectRef,
    pub expression: ObjectRef,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Backtest {
    pub schema: String,
    pub backtest_id: String,
    pub context: ObjectRef,
    pub factor: ObjectRef,
    pub engine: Engine,
    pub engine_version: String,
    pub sample: Sample,
    pub return_definition: String,
    pub deterministic_seed: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResultManifest {
    pub schema: String,
    pub job_id: String,
    pub specification: ObjectRef,
    pub engine: Engine,
    pub engine_version: String,
    pub metrics: Vec<Metric>,
    pub artifacts: Series,
    pub completed_at_ms: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Engine {
    PrimaryCrossSectional,
    AlphalensValidation,
    ZiplineValidation,
}

impl Engine {
    pub fn wire(self) -> i32 {
        (match self {
            Self::PrimaryCrossSectional => BacktestEngineKind::PrimaryCrossSectional,
            Self::AlphalensValidation => BacktestEngineKind::AlphalensValidation,
            Self::ZiplineValidation => BacktestEngineKind::ZiplineValidation,
        }) as i32
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metric {
    pub name: String,
    pub value: String,
    pub unit: String,
    pub estimator: String,
}

impl Metric {
    pub fn wire(&self) -> StoreResult<BacktestMetric> {
        Identifier::new(&self.name).map_err(|_| StoreError::Corrupt("metric name"))?;
        CanonicalDecimal::new(&self.value).map_err(|_| StoreError::Corrupt("metric decimal"))?;
        text(&self.unit)?;
        text(&self.estimator)?;
        Ok(BacktestMetric {
            name: self.name.clone(),
            value: Some(ExactDecimal {
                value: self.value.clone(),
            }),
            unit: self.unit.clone(),
            estimator: self.estimator.clone(),
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Series {
    pub factor_values: Artifact,
    pub target_positions: Artifact,
    pub orders: Artifact,
    pub fills: Artifact,
    pub nav: Artifact,
    pub simple_returns: Artifact,
    pub risk_exposures: Artifact,
    pub cost_ledger: Artifact,
}

impl Series {
    pub fn entries(&self) -> [&Artifact; 8] {
        [
            &self.factor_values,
            &self.target_positions,
            &self.orders,
            &self.fills,
            &self.nav,
            &self.simple_returns,
            &self.risk_exposures,
            &self.cost_ledger,
        ]
    }

    pub fn wire(&self) -> StoreResult<BacktestArtifacts> {
        Ok(BacktestArtifacts {
            factor_values: Some(self.factor_values.wire()?),
            target_positions: Some(self.target_positions.wire()?),
            orders: Some(self.orders.wire()?),
            fills: Some(self.fills.wire()?),
            nav: Some(self.nav.wire()?),
            simple_returns: Some(self.simple_returns.wire()?),
            risk_exposures: Some(self.risk_exposures.wire()?),
            cost_ledger: Some(self.cost_ledger.wire()?),
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub object: ObjectRef,
    pub schema: SchemaRef,
    pub media_type: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SchemaRef {
    pub name: String,
    pub version: u32,
    pub document: ObjectRef,
}

impl Artifact {
    pub fn wire(&self) -> StoreResult<ArtifactRef> {
        let content_digest = self.object.digest()?;
        let value = ArtifactRef {
            artifact_id: Some(ArtifactId {
                value: self.object.sha256.clone(),
            }),
            uri: format!("artifact://sha256/{}", &self.object.sha256[7..]),
            sha256: Some(digest(content_digest)),
            schema: Some(ArtifactSchemaReference {
                name: self.schema.name.clone(),
                version: self.schema.version,
                schema_sha256: Some(digest(self.schema.document.digest()?)),
            }),
            media_type: self.media_type.clone(),
            byte_size: self.object.byte_size,
            row_count: None,
            created_at: Some(timestamp(self.created_at_ms)?),
            manifest_sha256: None,
        };
        validate_artifact_ref(&value).map_err(|_| StoreError::Corrupt("artifact reference"))?;
        Ok(value)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SchemaDocument {
    pub schema: String,
    pub name: String,
    pub version: u32,
    pub media_type: String,
    pub columns: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Family {
    pub schema: String,
    pub algorithm: String,
    pub window_path: Vec<usize>,
    pub random_seed: String,
    pub backtest_seed: String,
    pub candidates: Vec<Candidate>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Candidate {
    pub window: u32,
    pub factor: ObjectRef,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Review {
    pub schema: String,
    pub job_id: String,
    pub result: ObjectRef,
    pub factor_spec_id: String,
    pub policy: Policy,
    pub library_sha256: String,
    pub eligible_observations: u64,
    pub valid_observations: u64,
    pub minimum_coverage_bps: u32,
    pub machine_rejection: String,
    pub semantic_accepted: bool,
    pub replacements: Vec<String>,
}

pub(super) fn digest(value: [u8; 32]) -> Sha256Digest {
    Sha256Digest {
        value: value.to_vec(),
    }
}

pub(super) fn parse_digest(value: &str) -> StoreResult<[u8; 32]> {
    ObjectRef {
        sha256: value.to_owned(),
        byte_size: 0,
    }
    .digest()
}

pub(super) fn timestamp(value: i64) -> StoreResult<prost_types::Timestamp> {
    if !(0..=253_402_300_799_999).contains(&value) {
        return Err(StoreError::Corrupt("manifest timestamp"));
    }
    Ok(prost_types::Timestamp {
        seconds: value / 1000,
        nanos: (value % 1000) as i32 * 1_000_000,
    })
}

pub(super) fn date(value: &str) -> StoreResult<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| StoreError::Corrupt("manifest date"))?;
    if parsed.format("%Y-%m-%d").to_string() != value {
        return Err(StoreError::Corrupt("non-canonical date"));
    }
    Ok(parsed)
}

pub(super) fn text(value: &str) -> StoreResult<()> {
    if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(StoreError::Corrupt("manifest text"));
    }
    Ok(())
}

pub(super) fn schema(actual: &str, expected: &str) -> StoreResult<()> {
    if actual != expected {
        return Err(StoreError::Corrupt("manifest schema"));
    }
    Ok(())
}
