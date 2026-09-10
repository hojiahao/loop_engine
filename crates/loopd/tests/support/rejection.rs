use loop_protocol::wire::jobs::v1::CompleteJobRequest;
use loop_protocol::wire::research::v1::{EnqueueBacktestRequest, ResearchJobBudget};
use loop_protocol::wire::v1::*;
use loopd::store::{PgJobStore, RoleCommand, SubmitJob};

use super::{NOW, backtest, context, research, timestamp};

pub fn command(index: u32) -> SubmitJob {
    let mut command = super::command(index);
    let (kind, input) = research::inputs().remove(2);
    command.specification.kind = kind as i32;
    command.specification.input = Some(input);
    command
}

pub fn role(key: &str) -> RoleCommand {
    let (_, job_specification::Input::Backtest(input)) = research::inputs().remove(2) else {
        unreachable!()
    };
    let budget = input.budget.unwrap();
    RoleCommand::Backtest(EnqueueBacktestRequest {
        context: Some(context(key)),
        input: Some(loop_protocol::wire::research::v1::BacktestInput {
            factor_spec_id: input.factor_spec_id,
            dataset: input.dataset,
            return_definition: input.return_definition,
            provenance: input.provenance,
            deterministic_seed: input.deterministic_seed,
            budget: Some(ResearchJobBudget {
                maximum_steps: budget.maximum_steps,
                maximum_input_tokens: budget.maximum_input_tokens,
                maximum_output_tokens: budget.maximum_output_tokens,
                maximum_cost: budget.maximum_cost,
                maximum_wall_time: budget.maximum_wall_time,
            }),
        }),
    })
}

pub async fn seed(store: &PgJobStore) -> CompleteJobRequest {
    let mut request = backtest::seed(store).await;
    let (_, job_specification::Input::Backtest(input)) = research::inputs().remove(2) else {
        unreachable!()
    };
    request.outcome = Some(JobOutcome {
        outcome: Some(job_outcome::Outcome::FactorRejection(FactorRejection {
            factor_spec_id: input.factor_spec_id,
            code: FactorRejectionCode::Performance as i32,
            reason: "fixture IS threshold not met".to_owned(),
            evidence: vec![super::artifact()],
            rejected_at: Some(timestamp(NOW)),
        })),
    });
    request
}
