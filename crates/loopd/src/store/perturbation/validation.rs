use std::collections::BTreeSet;

use loop_core::factor::FactorSpecId;
use loop_protocol::wire::v1::{
    BacktestJobInput, PerturbationReason, PerturbationSpace, PerturbationState, PerturbationStep,
    PerturbationWork,
};

use super::super::{StoreError, StoreResult, validate_id};

pub(super) fn space(space: &PerturbationSpace, input: &BacktestJobInput) -> StoreResult<()> {
    if space.algorithm != "window.ema-gradient-pcg64.v1"
        || space.dataset != input.dataset
        || space.provenance != input.provenance
        || space.backtest_seed != input.deterministic_seed
        || space
            .random_seed
            .as_ref()
            .is_none_or(|seed| seed.value.len() != 32)
        || !(2..=64).contains(&space.candidates.len())
    {
        return Err(StoreError::Invalid("perturbation space binding"));
    }
    let mut ids = BTreeSet::new();
    let mut previous = 0;
    for candidate in &space.candidates {
        let id = candidate
            .factor_spec_id
            .as_ref()
            .ok_or(StoreError::Invalid("candidate identity"))?;
        FactorSpecId::parse(&id.value).map_err(|_| StoreError::Invalid("candidate identity"))?;
        if candidate.window <= previous || candidate.window > 4096 || !ids.insert(&id.value) {
            return Err(StoreError::Invalid("candidate order or uniqueness"));
        }
        previous = candidate.window;
    }
    Ok(())
}

pub(super) fn state(state: &PerturbationState, space: &PerturbationSpace) -> StoreResult<()> {
    if state.version != 1
        || state.random_seed != space.random_seed
        || state.random_draws > 1_000_000_000
        || state.history.len() > 1024
        || !state.momentum.is_finite()
        || state.momentum.abs() > 2_000_000.0
        || !state.second_moment.is_finite()
        || !(0.0..=4e12).contains(&state.second_moment)
        || (state.history.is_empty() && (state.momentum != 0.0 || state.second_moment != 0.0))
    {
        return Err(StoreError::Corrupt("perturbation state"));
    }
    let mut jobs = BTreeSet::new();
    for item in &state.history {
        let job = item
            .source_job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("observation job"))?;
        validate_id(&job.value).map_err(|_| StoreError::Corrupt("observation job"))?;
        if !jobs.insert(&job.value)
            || !item.net_sharpe.is_finite()
            || item.net_sharpe.abs() > 1_000_000.0
            || item
                .candidate
                .as_ref()
                .is_none_or(|candidate| !space.candidates.contains(candidate))
        {
            return Err(StoreError::Corrupt("Sharpe history"));
        }
    }
    let mut proposed = BTreeSet::new();
    for id in &state.proposed_factor_ids {
        if !proposed.insert(&id.value)
            || !space
                .candidates
                .iter()
                .any(|candidate| candidate.factor_spec_id.as_ref() == Some(id))
        {
            return Err(StoreError::Corrupt("proposal history"));
        }
    }
    Ok(())
}

pub(super) fn transition(
    work: &PerturbationWork,
    space: &PerturbationSpace,
    step: &PerturbationStep,
) -> StoreResult<()> {
    let before = work
        .state
        .as_ref()
        .ok_or(StoreError::Corrupt("worker input state"))?;
    let after = step
        .state
        .as_ref()
        .ok_or(StoreError::Invalid("worker output state"))?;
    state(after, space).map_err(|_| StoreError::Invalid("worker output state"))?;
    let mut expected = before.history.clone();
    if let Some(observation) = &work.observation {
        match expected
            .iter()
            .find(|old| old.source_job_id == observation.source_job_id)
        {
            Some(old) if old != observation => {
                return Err(StoreError::Corrupt("source observation changed"));
            }
            None => expected.push(observation.clone()),
            _ => {}
        }
    }
    if after.history != expected
        || after.random_draws < before.random_draws
        || after.random_draws - before.random_draws > 32
        || (expected == before.history
            && (after.momentum != before.momentum || after.second_moment != before.second_moment))
    {
        return Err(StoreError::Invalid("worker history transition"));
    }
    let available: Vec<_> = space
        .candidates
        .iter()
        .filter(|candidate| {
            !expected
                .iter()
                .any(|item| item.candidate.as_ref() == Some(candidate))
                && candidate.factor_spec_id.as_ref().is_some_and(|id| {
                    !work.failed_factor_ids.contains(id) && !before.proposed_factor_ids.contains(id)
                })
        })
        .collect();
    let mut proposed = before.proposed_factor_ids.clone();
    if let Some(candidate) = &step.candidate {
        if !available.contains(&candidate)
            || after.random_draws == before.random_draws
            || !matches!(
                PerturbationReason::try_from(step.reason),
                Ok(PerturbationReason::Exploration | PerturbationReason::Gradient)
            )
        {
            return Err(StoreError::Invalid("worker proposal"));
        }
        proposed.push(
            candidate
                .factor_spec_id
                .clone()
                .ok_or(StoreError::Invalid("proposal id"))?,
        );
    } else if !available.is_empty()
        || step.reason != PerturbationReason::Exhausted as i32
        || after.random_draws != before.random_draws
    {
        return Err(StoreError::Invalid("worker exhaustion"));
    }
    if proposed != after.proposed_factor_ids {
        return Err(StoreError::Invalid("worker proposal history"));
    }
    Ok(())
}
