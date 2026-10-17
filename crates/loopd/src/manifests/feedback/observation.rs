use super::*;
use crate::manifests::LocalArtifacts;
use crate::manifests::numerical::{FrozenNumerical, VerifiedFamily};
use crate::manifests::portfolio::PreparedPortfolio;
use crate::store::BacktestPolicy;
use wire::{job_outcome, job_specification};

/// Source selected by a persisted earlier-round binding. The caller and store
/// still verify this exact record/result against the current database snapshot.
pub(crate) struct ObservationSource<'a> {
    pub run_id: &'a str,
    pub round: u32,
    pub before_round: u32,
    pub record: &'a wire::JobRecord,
    pub registered: &'a wire::BacktestResult,
    pub portfolio: Arc<PreparedPortfolio>,
    pub outputs: &'a LocalArtifacts,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Statistic {
    pub status: String,
    pub value: Option<f64>,
    pub reason: Option<String>,
    pub observations: u64,
}

/// Exact source identity and original result provenance for model context.
#[derive(Clone, Serialize)]
pub(crate) struct FeedbackEntry {
    pub round: u32,
    pub job_id: String,
    pub factor_spec_id: String,
    pub manifest: ObjectRef,
    pub statistics: ObjectRef,
    pub configuration_sha256: String,
    pub sharpe: Statistic,
}

/// Only actual portfolio files can construct an observation. Missing statistics
/// remain unavailable and never enter the numerical Sharpe observation type.
pub(crate) struct VerifiedObservation {
    pub(super) entry: FeedbackEntry,
    record: wire::JobRecord,
    result: wire::BacktestResult,
    portfolio: Arc<PreparedPortfolio>,
    files: Vec<Arc<VerifiedFile>>,
}

impl NumericalResolver {
    pub(crate) async fn observation(
        &self,
        frozen: &FrozenNumerical,
        source: ObservationSource<'_>,
    ) -> StoreResult<VerifiedObservation> {
        frozen.check()?;
        let record = source.record;
        let job = record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("feedback source job"))?;
        let Some(job_specification::Input::Backtest(input)) = &job.input else {
            return Err(StoreError::AdmissionDenied);
        };
        if record.state != wire::JobState::Succeeded as i32
            || job.run_id.as_ref().map(|id| id.value.as_str()) != Some(source.run_id)
            || source.round == 0
            || source.round >= source.before_round
            || source.before_round > 65
        {
            return Err(StoreError::AdmissionDenied);
        }
        let Some(job_outcome::Outcome::Success(success)) = record
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref())
        else {
            return Err(StoreError::AdmissionDenied);
        };
        let result = source.portfolio.resolve_result(job, success)?;
        if &result != source.registered || result.engine_version != "authorized-portfolio.1" {
            return Err(StoreError::Corrupt("feedback registered result"));
        }
        let work = source.portfolio.feedback_work();
        let factor = work
            .factor
            .as_ref()
            .ok_or(StoreError::Corrupt("feedback source factor"))?;
        let expected = frozen.evaluation(
            factor
                .expression
                .as_ref()
                .ok_or(StoreError::Corrupt("feedback AST"))?,
            wire::JobBudget::default(),
        )?;
        if expected.factor.as_ref() != Some(factor)
            || expected.provenance != work.provenance
            || expected.deterministic_seed != work.deterministic_seed
            || expected.dataset != input.dataset
            || input.factor_spec_id != factor.factor_spec_id
            || input.deterministic_seed != expected.deterministic_seed
        {
            return Err(StoreError::Corrupt("feedback frozen context"));
        }
        let policy = source.portfolio.feedback_policy()?;
        if policy
            .settings
            .get("statistics_profile")
            .map(String::as_str)
            != Some("daily-statistics.1")
        {
            return Err(StoreError::Corrupt("feedback statistics profile"));
        }
        let summary = unique_artifact(success, "loop.statistics_summary")?;
        let report = unique_artifact(success, "loop.authorized_portfolio")?;
        let reference = object(summary)?;
        let file = source
            .outputs
            .load(&reference, true, &mut ReadBudget::new())
            .await?;
        // Python's finite-float JSON spelling differs from serde_json for some
        // exponents. The artifact hash pins its actual bytes; do not reinterpret
        // a different serializer's spelling as missing or corrupt statistics.
        let value: serde_json::Value = serde_json::from_slice(file.bytes()?)
            .map_err(|_| StoreError::Corrupt("feedback statistics JSON"))?;
        let factor_id = factor
            .factor_spec_id
            .as_ref()
            .ok_or(StoreError::Corrupt("feedback factor ID"))?;
        let sharpe = statistic(&value, &result, &factor_id.value, &policy.settings)?;
        let manifest = object(report)?;
        if result
            .result_manifest_sha256
            .as_ref()
            .map(|hash| hash.value.as_slice())
            != Some(manifest.digest()?.as_slice())
        {
            return Err(StoreError::Corrupt("feedback manifest binding"));
        }
        let configuration = result
            .provenance
            .as_ref()
            .and_then(|value| value.configuration_sha256.as_ref())
            .ok_or(StoreError::Corrupt("feedback result provenance"))?;
        let mut files = frozen.guards();
        files.push(file);
        let evidence = VerifiedObservation {
            entry: FeedbackEntry {
                round: source.round,
                job_id: job
                    .job_id
                    .as_ref()
                    .ok_or(StoreError::Corrupt("feedback job ID"))?
                    .value
                    .clone(),
                factor_spec_id: factor_id.value.clone(),
                manifest,
                statistics: reference,
                configuration_sha256: format!("sha256:{}", hex(&configuration.value)),
                sharpe,
            },
            record: record.clone(),
            result,
            portfolio: source.portfolio,
            files,
        };
        evidence.check()?;
        Ok(evidence)
    }
}

impl VerifiedObservation {
    pub(crate) fn check(&self) -> StoreResult<()> {
        self.portfolio.check(
            self.record
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("feedback source job"))?,
        )?;
        check_files(&self.files)
    }

    pub(crate) fn entry(&self) -> &FeedbackEntry {
        &self.entry
    }

    pub(crate) fn source(&self) -> (&wire::JobRecord, &wire::BacktestResult) {
        (&self.record, &self.result)
    }

    /// A valid result outside the frozen family remains useful model feedback.
    pub(crate) fn eligible(
        &self,
        family: &VerifiedFamily,
    ) -> StoreResult<Option<wire::WindowObservation>> {
        self.check()?;
        family.check()?;
        let Some(value) = self.entry.sharpe.value else {
            return Ok(None);
        };
        // The legacy numerical algorithm has a finite score range. Preserve a
        // real extreme statistic in feedback, but do not clamp it into history.
        if value.abs() > 1_000_000.0 {
            return Ok(None);
        }
        let id = wire::FactorSpecId {
            value: self.entry.factor_spec_id.clone(),
        };
        Ok(family
            .candidate(&id)
            .cloned()
            .map(|candidate| wire::WindowObservation {
                source_job_id: Some(wire::JobId {
                    value: self.entry.job_id.clone(),
                }),
                candidate: Some(candidate),
                net_sharpe: value,
            }))
    }
}

fn unique_artifact<'a>(
    success: &'a wire::JobSuccess,
    name: &str,
) -> StoreResult<&'a wire::ArtifactRef> {
    let mut matches = success.outputs.iter().filter(|artifact| {
        artifact
            .schema
            .as_ref()
            .is_some_and(|schema| schema.name == name)
    });
    let artifact = matches
        .next()
        .ok_or(StoreError::Corrupt("missing feedback artifact"))?;
    if matches.next().is_some()
        || artifact
            .schema
            .as_ref()
            .is_none_or(|schema| schema.version != 1)
        || artifact.media_type != "application/json"
    {
        return Err(StoreError::Corrupt("feedback artifact schema"));
    }
    Ok(artifact)
}

pub(super) fn object(artifact: &wire::ArtifactRef) -> StoreResult<ObjectRef> {
    let reference = ObjectRef {
        sha256: artifact
            .artifact_id
            .as_ref()
            .ok_or(StoreError::Corrupt("feedback artifact ID"))?
            .value
            .clone(),
        byte_size: artifact.byte_size,
    };
    reference.digest()?;
    Ok(reference)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn statistic(
    summary: &serde_json::Value,
    result: &wire::BacktestResult,
    factor: &str,
    settings: &std::collections::BTreeMap<String, String>,
) -> StoreResult<Statistic> {
    for (key, expected) in [
        ("schema", "loop.statistics-summary/v1"),
        ("factor_spec_id", factor),
        ("benchmark", "zero-daily-return"),
        (
            "sharpe_annualization",
            "sqrt(252)-descriptive-no-serial-correction",
        ),
        ("return_basis", "cost-adjusted-simple-nav-return"),
        ("sharpe_dispersion", "sample-standard-deviation-ddof-1"),
    ] {
        if summary.get(key).and_then(serde_json::Value::as_str) != Some(expected) {
            return Err(StoreError::Corrupt("feedback Sharpe semantics"));
        }
    }
    for key in [
        "groups",
        "minimum_cross_section",
        "minimum_sessions",
        "hac_lags",
        "pbo_blocks",
    ] {
        let expected = settings
            .get(key)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(StoreError::Corrupt("feedback statistics policy"))?;
        if summary
            .get("policy")
            .and_then(|policy| policy.get(key))
            .and_then(serde_json::Value::as_u64)
            != Some(expected)
        {
            return Err(StoreError::Corrupt("feedback statistics policy binding"));
        }
    }
    let parsed: Statistic = serde_json::from_value(
        summary
            .get("sharpe_annualized")
            .cloned()
            .ok_or(StoreError::Corrupt("feedback Sharpe"))?,
    )
    .map_err(|_| StoreError::Corrupt("feedback Sharpe shape"))?;
    let count = summary
        .get("return_observations")
        .and_then(serde_json::Value::as_u64)
        .ok_or(StoreError::Corrupt("feedback return count"))?;
    if count != parsed.observations
        || count > 8191
        || summary
            .get("observed_sessions")
            .and_then(serde_json::Value::as_u64)
            != Some(count + 1)
        || summary
            .get("production_eligible")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
    {
        return Err(StoreError::Corrupt("feedback Sharpe observations"));
    }
    let metrics: Vec<_> = result
        .metrics
        .iter()
        .filter(|metric| metric.name == "sharpe_annualized")
        .collect();
    match (
        &*parsed.status,
        parsed.value,
        parsed.reason.as_deref(),
        metrics.as_slice(),
    ) {
        ("available", Some(value), None, [metric])
            if value.is_finite()
                && settings
                    .get("minimum_sessions")
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_some_and(|minimum| count >= minimum)
                && metric.unit == "ratio"
                && metric.estimator == "daily-statistics.1"
                && metric
                    .value
                    .as_ref()
                    .and_then(|value| value.value.parse::<f64>().ok())
                    == Some(value) => {}
        ("unavailable", None, Some(reason), [])
            if !reason.is_empty()
                && reason.len() <= 128
                && !reason.chars().any(char::is_control) => {}
        _ => return Err(StoreError::Corrupt("feedback registered Sharpe")),
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests;
