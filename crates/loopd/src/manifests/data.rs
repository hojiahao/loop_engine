use chrono::Datelike;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use loop_protocol::wire::v1::{ArtifactRef, JobSpecification, SampleRole, job_specification};
use prost::Message;
use sha2::{Digest, Sha256};

use super::files::VerifiedFile;
use super::loading::{Materializer, sorted};
use super::{LocalArtifacts, ObjectRef, model};
use crate::store::{StoreError, StoreResult};

/// Internally constructed proof; a caller cannot substitute an unverified list.
pub(crate) struct DataEvidence {
    job_checksum: [u8; 32],
    pub(crate) manifest: ObjectRef,
    artifacts: Vec<(ArtifactRef, Arc<VerifiedFile>)>,
    files: Vec<Arc<VerifiedFile>>,
}

impl DataEvidence {
    pub(crate) fn check(&self, job: &JobSpecification) -> StoreResult<()> {
        if Sha256::digest(job.encode_to_vec()).as_slice() != self.job_checksum {
            return Err(StoreError::Corrupt("data job binding"));
        }
        for file in &self.files {
            file.check()?;
        }
        Ok(())
    }

    pub(crate) fn artifacts(&self) -> Vec<ArtifactRef> {
        self.artifacts
            .iter()
            .map(|(artifact, _)| artifact.clone())
            .collect()
    }

    pub(crate) async fn copy_to(&self, directory: &Path) -> StoreResult<()> {
        use std::os::unix::fs::PermissionsExt;
        for (artifact, file) in &self.artifacts {
            let digest = artifact
                .sha256
                .as_ref()
                .ok_or(StoreError::Corrupt("data artifact digest"))?;
            let name: String = digest
                .value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let target = directory.join(name);
            file.copy_to(&target).await?;
            tokio::fs::set_permissions(target, std::fs::Permissions::from_mode(0o444)).await?;
        }
        Ok(())
    }
}

pub(crate) async fn resolve(
    source: &LocalArtifacts,
    reference: &ObjectRef,
    job: &JobSpecification,
    protected: bool,
) -> StoreResult<DataEvidence> {
    let mut materializer = Materializer::new(source, &[]);
    let data: model::Dataset = materializer.json(reference).await?;
    let actual = data.reference(reference)?;
    let expected = match (&job.input, protected) {
        (Some(job_specification::Input::Backtest(input)), false) => input.dataset.as_ref(),
        (Some(job_specification::Input::FactorEvaluation(input)), false) => input.dataset.as_ref(),
        (Some(job_specification::Input::HoldoutBacktest(input)), true) => {
            model::schema(&data.schema, "loop.protected-dataset/v1")?;
            if !matches!(data.quality, model::Quality::Synthetic) {
                return Err(StoreError::AdmissionDenied);
            }
            let spec = input
                .frozen_backtest_spec
                .as_ref()
                .ok_or(StoreError::Corrupt("protected backtest"))?;
            let sample = spec
                .sample
                .as_ref()
                .ok_or(StoreError::Corrupt("protected sample"))?;
            let (role, lower, upper) = match data.sample.role {
                model::SampleRole::FirstLockedConfirmation => (
                    SampleRole::FirstLockedConfirmation,
                    "2021-01-01",
                    "2024-12-31",
                ),
                model::SampleRole::SecondLockedHistoricalHoldout => (
                    SampleRole::SecondLockedHistoricalHoldout,
                    "2025-01-01",
                    "2026-08-31",
                ),
                _ => return Err(StoreError::AdmissionDenied),
            };
            let start = model::date(&data.sample.start)?;
            let end = model::date(&data.sample.end)?;
            if start > end
                || start < model::date(lower)?
                || end > model::date(upper)?
                || sample.role != role as i32
                || sample
                    .start_inclusive
                    .as_ref()
                    .map(|date| (date.year, date.month, date.day))
                    != Some((start.year(), start.month(), start.day()))
                || sample
                    .end_inclusive
                    .as_ref()
                    .map(|date| (date.year, date.month, date.day))
                    != Some((end.year(), end.month(), end.day()))
                || spec.snapshot_ids != actual.snapshot_ids
                || spec
                    .provenance
                    .as_ref()
                    .and_then(|provenance| provenance.data_manifest_sha256.as_ref())
                    != actual.manifest_sha256.as_ref()
            {
                return Err(StoreError::Corrupt("protected data binding"));
            }
            None
        }
        _ => return Err(StoreError::AdmissionDenied),
    };
    if !protected {
        model::schema(&data.schema, "loop.development-dataset/v1")?;
        data.sample.validate()?;
        if expected != Some(&actual) {
            return Err(StoreError::Corrupt("development data binding"));
        }
    }
    sorted(
        data.snapshots
            .iter()
            .map(|snapshot| snapshot.snapshot_id.as_str()),
        1,
        128,
    )?;
    let mut identities = BTreeSet::new();
    let mut artifacts = Vec::new();
    let mut bytes = 0_u64;
    let latest = model::date(&data.sample.end)?
        .and_hms_opt(23, 59, 59)
        .ok_or(StoreError::Corrupt("data end time"))?
        .and_utc()
        .timestamp_millis()
        + 999;
    for snapshot in &data.snapshots {
        model::text(&snapshot.source)?;
        model::text(&snapshot.dataset)?;
        model::text(&snapshot.entitlement)?;
        model::timestamp(snapshot.known_through_ms)?;
        if snapshot.known_through_ms > latest || snapshot.artifacts.is_empty() {
            return Err(StoreError::Corrupt("data information boundary"));
        }
        for artifact in &snapshot.artifacts {
            if !identities.insert(artifact.object.sha256.clone()) {
                return Err(StoreError::Corrupt("duplicate data artifact"));
            }
            bytes = bytes
                .checked_add(artifact.object.byte_size)
                .ok_or(StoreError::Invalid("data view byte budget"))?;
            if artifacts.len() >= 128 || bytes > 256 * 1024 * 1024 {
                return Err(StoreError::Invalid("data view budget"));
            }
            materializer.artifact(artifact).await?;
            let file = materializer.object(&artifact.object, false).await?;
            artifacts.push((artifact.wire()?, file));
        }
    }
    let evidence = DataEvidence {
        job_checksum: Sha256::digest(job.encode_to_vec()).into(),
        manifest: reference.clone(),
        artifacts,
        files: materializer.files,
    };
    evidence.check(job)?;
    Ok(evidence)
}
