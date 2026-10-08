use loop_protocol::wire::{discovery::v1 as wire, v1};

use crate::store::{ModelStep, ModelStepState, StoreError, StoreResult};

/// Project only verified durable metadata. This never resolves a plan or exposes
/// a candidate; execution adds those only after checking the frozen plan.
pub(in crate::runtime) fn metadata(
    job: &v1::JobRecord,
    history: &[ModelStep],
) -> StoreResult<wire::DiscoveryStepView> {
    let specification = job
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("discovery specification"))?;
    if specification.kind != v1::JobKind::Discovery as i32
        || history.iter().any(|step| step.job != *job)
    {
        return Err(StoreError::Corrupt("model view revision"));
    }
    let step = history.last();
    let state = match step.map(|step| step.state) {
        None => wire::DiscoveryStepState::Unspecified,
        Some(ModelStepState::Reserved) => wire::DiscoveryStepState::Reserved,
        Some(ModelStepState::Dispatched) => wire::DiscoveryStepState::Dispatched,
        Some(ModelStepState::Ambiguous) => wire::DiscoveryStepState::Ambiguous,
        Some(ModelStepState::Completed) => wire::DiscoveryStepState::Completed,
    };
    let mut input = 0_u64;
    let mut output = 0_u64;
    let mut cost = 0_u64;
    for step in history {
        input = input
            .checked_add(step.reserved_input)
            .ok_or(StoreError::Corrupt("input reservation sum"))?;
        output = output
            .checked_add(step.reserved_output)
            .ok_or(StoreError::Corrupt("output reservation sum"))?;
        cost = cost
            .checked_add(step.reserved_nano_usd)
            .ok_or(StoreError::Corrupt("cost reservation sum"))?;
    }
    Ok(wire::DiscoveryStepView {
        job: Some(wire::DiscoveryJobHandle {
            job_id: specification.job_id.clone(),
            status: job.state,
            revision: job.revision,
            submitted_at: specification.submitted_at,
            updated_at: job.updated_at,
        }),
        state: state as i32,
        candidate: None,
        reserved_cost: step.map(|_| v1::Money {
            currency_code: "USD".into(),
            amount: Some(v1::ExactDecimal {
                value: super::execution::usd(cost),
            }),
        }),
        reserved_input_tokens: input,
        reserved_output_tokens: output,
        plan_verified: false,
    })
}
