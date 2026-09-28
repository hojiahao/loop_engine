//! Verified numerical trials; coverage success is not portfolio admission.

use loop_protocol::wire::v1::{
    FactorEvaluationResult, JobRecord, JobSpecification, JobState, Sha256Digest, job_outcome,
    job_specification,
};
use prost::Message;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use super::postgres::{record_from_row, verified_blob};
use super::{PgJobStore, StoreError, StoreResult};

/// Immutable completion evidence returned with a trial. This is a historical
/// metadata view, not current-file attestation or permission to read artifacts.
#[derive(Clone, PartialEq, Message)]
pub struct EvaluationTrial {
    /// Actual installed-worker result, bound to the completed job and artifacts.
    #[prost(message, optional, tag = "1")]
    pub result: Option<FactorEvaluationResult>,
    /// Integer threshold resolved from the factor's frozen evaluation policy.
    #[prost(uint32, tag = "2")]
    pub minimum_coverage_bps: u32,
    /// `ready_for_backtest` or `insufficient_coverage`; never `admitted`.
    #[prost(string, tag = "3")]
    pub disposition: String,
}

pub(crate) fn coverage_passes(eligible: u64, valid: u64, minimum: u32) -> bool {
    eligible > 0
        && valid <= eligible
        && (1..=10_000).contains(&minimum)
        && u128::from(valid) * 10_000 >= u128::from(eligible) * u128::from(minimum)
}

#[derive(Serialize)]
struct Context<'a> {
    factor_spec_id: &'a str,
    snapshot_ids: Vec<&'a str>,
    data_manifest: &'a [u8],
    provenance: [&'a [u8]; 6],
    seed: &'a [u8],
}

fn digest(value: Option<&Sha256Digest>) -> StoreResult<&[u8]> {
    value
        .filter(|v| v.value.len() == 32)
        .map(|v| v.value.as_slice())
        .ok_or(StoreError::Invalid("evaluation context digest"))
}

fn context_key(job: &JobSpecification) -> StoreResult<Option<[u8; 32]>> {
    let Some(job_specification::Input::FactorEvaluation(input)) = &job.input else {
        return Ok(None);
    };
    // Legacy metadata-only envelopes remain readable, but cannot execute the
    // installed worker or claim new numerical evidence.
    let (Some(provenance), Some(seed)) = (&input.provenance, &input.deterministic_seed) else {
        return Ok(None);
    };
    let data = input
        .dataset
        .as_ref()
        .ok_or(StoreError::Invalid("evaluation data"))?;
    let id = input
        .factor
        .as_ref()
        .and_then(|f| f.factor_spec_id.as_ref())
        .ok_or(StoreError::Invalid("evaluation factor"))?;
    let context = Context {
        factor_spec_id: &id.value,
        snapshot_ids: data
            .snapshot_ids
            .iter()
            .map(|id| id.value.as_str())
            .collect(),
        data_manifest: digest(data.manifest_sha256.as_ref())?,
        provenance: [
            digest(provenance.source_code_sha256.as_ref())?,
            digest(provenance.operator_registry_sha256.as_ref())?,
            digest(provenance.configuration_sha256.as_ref())?,
            digest(provenance.data_manifest_sha256.as_ref())?,
            digest(provenance.trading_calendar_sha256.as_ref())?,
            digest(provenance.environment_sha256.as_ref())?,
        ],
        seed: digest(Some(seed))?,
    };
    let mut hash = Sha256::new();
    hash.update(b"loop.factor-evaluation-context/v1\0");
    hash.update(
        serde_json::to_vec(&context).map_err(|_| StoreError::Invalid("evaluation context"))?,
    );
    Ok(Some(hash.finalize().into()))
}

pub(super) fn completed(record: &JobRecord) -> bool {
    record.state == JobState::Succeeded as i32
        && matches!(
            record.specification.as_ref().and_then(|j| j.input.as_ref()),
            Some(job_specification::Input::FactorEvaluation(_))
        )
}

pub(super) async fn record_completion(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
) -> StoreResult<()> {
    if !completed(record) {
        return Ok(());
    }
    let proof = store
        .evaluation_evidence
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    proof.check(record)?;
    let trial = EvaluationTrial {
        result: Some(proof.result.clone()),
        minimum_coverage_bps: proof.minimum_coverage_bps,
        disposition: if coverage_passes(
            proof.result.eligible_observations,
            proof.result.valid_observations,
            proof.minimum_coverage_bps,
        ) {
            "ready_for_backtest"
        } else {
            "insufficient_coverage"
        }
        .to_owned(),
    };
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("evaluation job"))?;
    let key = context_key(job)?.ok_or(StoreError::Corrupt("evaluation provenance"))?;
    let blob = trial.encode_to_vec();
    sqlx::query(
        "INSERT INTO factor_evaluations
        (job_id, job_revision, record_sha256, context_sha256, evidence_blob, evidence_sha256)
        VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(
        &job.job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("evaluation ID"))?
            .value,
    )
    .bind(record.revision as i64)
    .bind(Sha256::digest(record.encode_to_vec()).as_slice())
    .bind(key.as_slice())
    .bind(&blob)
    .bind(Sha256::digest(&blob).as_slice())
    .execute(&mut **transaction)
    .await?;
    #[cfg(test)]
    super::crash_tests::fault_point("evaluation_after_insert").await;
    Ok(())
}

fn verify(row: &PgRow, record: &JobRecord) -> StoreResult<Option<EvaluationTrial>> {
    if !completed(record)
        || row.try_get::<i64, _>("job_revision")? != record.revision as i64
        || row.try_get::<Vec<u8>, _>("evaluation_record_sha256")?
            != Sha256::digest(record.encode_to_vec()).as_slice()
    {
        return Err(StoreError::Corrupt("evaluation record binding"));
    }
    let Some(key) = row.try_get::<Option<Vec<u8>>, _>("context_sha256")? else {
        return Ok(None); // Immutable pre-migration success, not admission evidence.
    };
    let job = record
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("evaluation job"))?;
    if context_key(job)?.as_ref().map(|k| k.as_slice()) != Some(key.as_slice()) {
        return Err(StoreError::Corrupt("evaluation context binding"));
    }
    let trial =
        EvaluationTrial::decode(verified_blob(row, "evidence_blob", "evidence_sha256")?.as_slice())
            .map_err(|_| StoreError::Corrupt("evaluation evidence"))?;
    let result = trial
        .result
        .as_ref()
        .ok_or(StoreError::Corrupt("evaluation result"))?;
    let Some(job_specification::Input::FactorEvaluation(input)) = &job.input else {
        return Err(StoreError::Corrupt("evaluation input"));
    };
    let factor = input
        .factor
        .as_ref()
        .ok_or(StoreError::Corrupt("evaluation factor"))?;
    let expected = if coverage_passes(
        result.eligible_observations,
        result.valid_observations,
        trial.minimum_coverage_bps,
    ) {
        "ready_for_backtest"
    } else {
        "insufficient_coverage"
    };
    let success = match record.outcome.as_ref().and_then(|o| o.outcome.as_ref()) {
        Some(job_outcome::Outcome::Success(value)) => value,
        _ => return Err(StoreError::Corrupt("evaluation outcome")),
    };
    if result.job_id != job.job_id
        || result.factor_spec_id != factor.factor_spec_id
        || result.expression_id != factor.expression_id
        || result.provenance != input.provenance
        || result.deterministic_seed != input.deterministic_seed
        || !(1..=10_000).contains(&trial.minimum_coverage_bps)
        || result.valid_observations > result.eligible_observations
        || trial.disposition != expected
        || success.outputs.len() != 2
        || result.values.as_ref() != success.outputs.first()
        || result.manifest.as_ref() != success.outputs.get(1)
    {
        return Err(StoreError::Corrupt("evaluation evidence binding"));
    }
    Ok(Some(trial))
}

pub(super) async fn read(
    transaction: &mut Transaction<'_, Postgres>,
    record: &JobRecord,
) -> StoreResult<Option<EvaluationTrial>> {
    if !completed(record) {
        return Ok(None);
    }
    let id = record
        .specification
        .as_ref()
        .and_then(|j| j.job_id.as_ref())
        .ok_or(StoreError::Corrupt("evaluation ID"))?;
    let row = sqlx::query(
        "SELECT *, record_sha256 AS evaluation_record_sha256
        FROM factor_evaluations WHERE job_id = $1",
    )
    .bind(&id.value)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(StoreError::Corrupt("missing evaluation projection"))?;
    verify(&row, record)
}

pub(super) async fn check_previous(
    transaction: &mut Transaction<'_, Postgres>,
    job: &JobSpecification,
) -> StoreResult<()> {
    let Some(key) = context_key(job)? else {
        return Ok(());
    };
    let row = sqlx::query(
        "SELECT j.*, e.job_revision, e.record_sha256 AS evaluation_record_sha256,
        e.context_sha256, e.evidence_blob, e.evidence_sha256
        FROM factor_evaluations e JOIN jobs j USING (job_id)
        WHERE e.context_sha256 = $1 ORDER BY e.job_id LIMIT 1",
    )
    .bind(key.as_slice())
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(row) = row {
        let prior = record_from_row(&row)?;
        let trial = verify(&row, &prior)?.ok_or(StoreError::Corrupt("legacy evaluation lookup"))?;
        return Err(if trial.disposition == "insufficient_coverage" {
            StoreError::PreviouslyRejected
        } else {
            StoreError::AlreadyEvaluated
        });
    }
    Ok(())
}

pub(super) async fn check_admission(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    principal: &loop_protocol::wire::v1::Actor,
    backtest: &JobRecord,
    result: &loop_protocol::wire::v1::BacktestResult,
    evidence: &super::AdmissionEvidence,
) -> StoreResult<()> {
    // Legacy synthetic policy doubles remain useful for decision/CAS goldens.
    // The deployment's actual-file resolver requires a v2 link before this call.
    let Some(link) = &evidence.evaluation else {
        return Ok(());
    };
    super::validate_id(&link.job_id)?;
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
        .bind(&link.job_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(StoreError::AdmissionDenied)?;
    let record = record_from_row(&row)?;
    store
        .admission
        .authorize_job_command("loop.factors.read_trials", principal, &record)?;
    let trial = super::library::trials::verify(transaction, &record).await?;
    let numerical = trial.evaluation.ok_or(StoreError::AdmissionDenied)?;
    let computed = numerical
        .result
        .as_ref()
        .ok_or(StoreError::Corrupt("admission evaluation"))?;
    let Some(job_specification::Input::Backtest(target)) = backtest
        .specification
        .as_ref()
        .and_then(|j| j.input.as_ref())
    else {
        return Err(StoreError::AdmissionDenied);
    };
    let Some(job_specification::Input::FactorEvaluation(input)) =
        record.specification.as_ref().and_then(|j| j.input.as_ref())
    else {
        return Err(StoreError::AdmissionDenied);
    };
    let factor = input
        .factor
        .as_ref()
        .ok_or(StoreError::Corrupt("admission evaluated factor"))?;
    let provenance = input
        .provenance
        .as_ref()
        .ok_or(StoreError::Corrupt("evaluation provenance"))?;
    let current = target
        .provenance
        .as_ref()
        .ok_or(StoreError::Corrupt("backtest provenance"))?;
    if link.manifest_sha256.len() != 32
        || computed
            .manifest
            .as_ref()
            .and_then(|a| a.sha256.as_ref())
            .map(|d| d.value.as_slice())
            != Some(link.manifest_sha256.as_slice())
        || factor.factor_spec_id != target.factor_spec_id
        || input.dataset != target.dataset
        || input.deterministic_seed != target.deterministic_seed
        || provenance.source_code_sha256 != current.source_code_sha256
        || provenance.environment_sha256 != current.environment_sha256
        || provenance.operator_registry_sha256 != current.operator_registry_sha256
        || provenance.data_manifest_sha256 != current.data_manifest_sha256
        || provenance.trading_calendar_sha256 != current.trading_calendar_sha256
        || !same_values(
            computed.values.as_ref(),
            result
                .artifacts
                .as_ref()
                .and_then(|a| a.factor_values.as_ref()),
        )
        || evidence.policy.as_ref()
            != factor
                .frozen_policy
                .as_ref()
                .and_then(|p| p.evaluation_policy.as_ref())
        || evidence.eligible_observations != computed.eligible_observations
        || evidence.valid_observations != computed.valid_observations
        || evidence.minimum_coverage_bps != numerical.minimum_coverage_bps
    {
        return Err(StoreError::Corrupt("admission numerical lineage"));
    }
    Ok(())
}

fn same_values(
    left: Option<&loop_protocol::wire::v1::ArtifactRef>,
    right: Option<&loop_protocol::wire::v1::ArtifactRef>,
) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    // Transport URI, publication time and optional row-count metadata are not
    // content identity. The verified bytes, encoding and schema must coincide.
    left.artifact_id == right.artifact_id
        && left.sha256 == right.sha256
        && left.byte_size == right.byte_size
        && left.media_type == right.media_type
        && left.schema == right.schema
        && right
            .row_count
            .is_none_or(|count| Some(count) == left.row_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as support;

    fn job() -> JobSpecification {
        let mut job = support::command(1).specification;
        let (kind, mut input) = support::research::inputs().remove(1);
        let job_specification::Input::FactorEvaluation(evaluation) = &mut input else {
            unreachable!()
        };
        evaluation.provenance = support::backtest::result().provenance;
        evaluation.deterministic_seed = Some(support::digest(8));
        job.kind = kind as i32;
        job.input = Some(input);
        job
    }

    #[test]
    // Scenario: coverage boundaries are exact.
    fn coverage_boundaries() {
        assert!(coverage_passes(6, 4, 6666));
        assert!(!coverage_passes(6, 4, 6667));
        assert!(!coverage_passes(0, 0, 1));
        assert!(!coverage_passes(3, 4, 1));
        assert!(!coverage_passes(1, 1, 0));
        assert!(!coverage_passes(1, 1, 10_001));
        assert!(!coverage_passes(u64::MAX, u64::MAX - 1, 10_000));
        assert!(coverage_passes(u64::MAX, u64::MAX - 1, 9999));
    }

    #[test]
    // Scenario: orchestration metadata cannot bypass memory.
    fn orchestration_metadata_memory() {
        let first = job();
        let mut other = first.clone();
        other.job_id.as_mut().unwrap().value = "job.other".to_owned();
        other.run_id.as_mut().unwrap().value = "run.other".to_owned();
        other.idempotency_key.as_mut().unwrap().value = "other".to_owned();
        let Some(job_specification::Input::FactorEvaluation(input)) = &mut other.input else {
            unreachable!()
        };
        input.budget.as_mut().unwrap().maximum_steps += 1;
        assert_eq!(context_key(&first).unwrap(), context_key(&other).unwrap());
    }

    #[test]
    // Scenario: frozen context changes do not share failures.
    fn frozen_context_failures() {
        let original = job();
        let expected = context_key(&original).unwrap().unwrap();
        for component in 0..10 {
            let mut modified = original.clone();
            let Some(job_specification::Input::FactorEvaluation(input)) = &mut modified.input
            else {
                unreachable!()
            };
            let provenance = input.provenance.as_mut().unwrap();
            match component {
                0 => {
                    input
                        .factor
                        .as_mut()
                        .unwrap()
                        .factor_spec_id
                        .as_mut()
                        .unwrap()
                        .value = format!("sha256:{}", "f".repeat(64))
                }
                1 => {
                    input.dataset.as_mut().unwrap().snapshot_ids[0].value =
                        "snapshot.other".to_owned()
                }
                2 => {
                    input
                        .dataset
                        .as_mut()
                        .unwrap()
                        .manifest_sha256
                        .as_mut()
                        .unwrap()
                        .value[0] ^= 1
                }
                3 => input.deterministic_seed.as_mut().unwrap().value[0] ^= 1,
                4 => provenance.source_code_sha256.as_mut().unwrap().value[0] ^= 1,
                5 => provenance.operator_registry_sha256.as_mut().unwrap().value[0] ^= 1,
                6 => provenance.configuration_sha256.as_mut().unwrap().value[0] ^= 1,
                7 => provenance.data_manifest_sha256.as_mut().unwrap().value[0] ^= 1,
                8 => provenance.trading_calendar_sha256.as_mut().unwrap().value[0] ^= 1,
                _ => provenance.environment_sha256.as_mut().unwrap().value[0] ^= 1,
            }
            assert_ne!(
                context_key(&modified).unwrap().unwrap(),
                expected,
                "component {component}"
            );
        }
    }
}
