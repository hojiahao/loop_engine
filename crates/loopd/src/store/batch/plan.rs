use loop_core::audit::Sha256Digest as CanonicalDigest;
use loop_core::holdout::{HoldoutJobBudget, parse_canonical_holdout_evaluation_plan};
use loop_protocol::holdout::validate_holdout_period;
use loop_protocol::wire::v1::{BacktestSpec, ExactDecimal, JobBudget, Money};

use super::super::grant::{resolve, state};
use super::{PgJobStore, StoreError, StoreResult};

pub(super) async fn materialize(
    store: &PgJobStore,
    grant: &state::PersistedGrant,
    now: i64,
) -> StoreResult<Vec<(BacktestSpec, JobBudget)>> {
    let resolved = resolve::resolve_freeze(store, &grant.freeze, &grant.period, now)?;
    let period = grant
        .period
        .record
        .period
        .as_ref()
        .ok_or(StoreError::Corrupt("batch period"))?;
    let canonical = validate_holdout_period(period, &grant.period.canonical_bytes)?;
    let plan = parse_canonical_holdout_evaluation_plan(
        &resolved.canonical_plan,
        &canonical,
        &resolved.backtest_schema_sha256,
        &resolved.backtest_artifacts,
    )?;
    let mut output = Vec::with_capacity(plan.value.entries.len());
    let mut total_bytes = 0_usize;
    for entry in &plan.value.entries {
        // Yield between bounded entries so cancellation and the command deadline can run.
        tokio::task::yield_now().await;
        let bytes = resolved
            .backtest_artifacts
            .get(&entry.backtest_spec_artifact.sha256)
            .ok_or(StoreError::AdmissionDenied)?;
        let spec = store
            .holdout_policy
            .materialize_backtest(&resolved.reference, entry, bytes)?;
        let expected = CanonicalDigest::parse(&entry.backtest_spec_artifact.sha256)
            .map_err(|_| StoreError::Corrupt("batch artifact digest"))?;
        let provenance = spec
            .provenance
            .as_ref()
            .ok_or(StoreError::Invalid("batch provenance"))?;
        if spec.factor_spec_id.as_ref().map(|id| id.value.as_str())
            != Some(entry.factor_spec_id.as_str())
            || resolve::digest(spec.canonical_spec_sha256.as_ref())? != expected.as_bytes()
            || spec.sample != period.sample
            || spec.snapshot_ids != period.snapshot_ids
            || provenance.source_code_sha256 != resolved.reference.source_tree_sha256
            || provenance.configuration_sha256 != resolved.reference.configuration_sha256
            || provenance.data_manifest_sha256 != resolved.reference.data_manifest_sha256
        {
            return Err(StoreError::Invalid("materialized backtest binding"));
        }
        total_bytes = total_bytes
            .checked_add(prost::Message::encoded_len(&spec))
            .ok_or(StoreError::Invalid("batch metadata size"))?;
        if total_bytes > 64 * 1024 * 1024 {
            return Err(StoreError::Invalid("batch metadata size"));
        }
        output.push((spec, budget(&entry.job_budget)?));
    }
    Ok(output)
}

fn budget(value: &HoldoutJobBudget) -> StoreResult<JobBudget> {
    let parse = |value: &str| {
        value
            .parse::<u64>()
            .map_err(|_| StoreError::Invalid("batch budget"))
    };
    let nanos = value
        .maximum_wall_time_ns
        .parse::<u128>()
        .map_err(|_| StoreError::Invalid("batch wall time"))?;
    Ok(JobBudget {
        maximum_steps: u32::try_from(parse(&value.maximum_steps)?)
            .map_err(|_| StoreError::Invalid("batch steps"))?,
        maximum_input_tokens: parse(&value.maximum_input_tokens)?,
        maximum_output_tokens: parse(&value.maximum_output_tokens)?,
        maximum_cost: Some(Money {
            amount: Some(ExactDecimal {
                value: value.maximum_cost.amount.clone(),
            }),
            currency_code: value.maximum_cost.currency_code.clone(),
        }),
        maximum_wall_time: Some(prost_types::Duration {
            seconds: i64::try_from(nanos / 1_000_000_000)
                .map_err(|_| StoreError::Invalid("batch wall time"))?,
            nanos: (nanos % 1_000_000_000) as i32,
        }),
    })
}
