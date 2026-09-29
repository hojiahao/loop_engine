use std::time::Duration;

use loop_protocol::wire::{discovery::v1 as wire, provider::v1::InvokeModelRequest, v1};
use uuid::Uuid;

use super::DiscoveryExecutor;
use crate::runtime::{RuntimeAuthority, model_codec};
use crate::store::{
    ModelStep, ModelStepCommand, ModelStepState, PgJobStore, StoreError, StoreResult,
    model_duration,
};

impl DiscoveryExecutor {
    pub(crate) async fn execute(
        &self,
        store: &PgJobStore,
        authority: &RuntimeAuthority,
        actor: &v1::Actor,
        job: v1::JobRecord,
        expected_revision: u64,
    ) -> StoreResult<wire::DiscoveryStepView> {
        if expected_revision != job.revision {
            return Err(StoreError::RevisionConflict);
        }
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery specification"))?;
        let plan = self.plan(specification)?;
        let evidence =
            crate::manifests::data::resolve(&self.data, &plan.data, specification, false).await?;
        let job_id = &specification
            .job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery job ID"))?
            .value;
        let prior = store.model_step(actor, job_id).await?;
        let mut step = if let Some(step) = prior {
            if step.job.revision != expected_revision {
                return Err(StoreError::RevisionConflict);
            }
            check_request(plan, &step)?;
            if step.state == ModelStepState::Completed {
                return self.view(&step.job, Some(&step));
            }
            // A second RPC cannot borrow another in-flight handler's lease.
            let mut takeover = command(authority, actor, &step.job)?;
            takeover.lease_id = None;
            store
                .takeover_model(
                    actor,
                    takeover,
                    prost_types::Duration {
                        seconds: 120,
                        nanos: 0,
                    },
                )
                .await?
        } else {
            let mut invocation = plan.invocation.clone();
            let context = context(authority, actor, job_id)?;
            invocation.request_id = context.request_id.clone();
            let request = InvokeModelRequest {
                context: Some(context),
                invocation: Some(invocation),
            };
            store
                .reserve_model(actor, command(authority, actor, &job)?, request)
                .await?
        };
        plan.check()?;
        evidence.check(specification)?;
        let send = if step.state == ModelStepState::Reserved {
            let dispatch = store
                .dispatch_model(actor, command(authority, actor, &step.job)?)
                .await?;
            step = dispatch.step;
            dispatch.send
        } else {
            false
        };
        let timeout = remaining(authority, &step, send)?;
        let result = if send {
            self.provider
                .invoke(step.request.clone(), timeout)
                .await
                .map(Some)
        } else {
            self.provider
                .lookup(&step, context(authority, actor, job_id)?, timeout)
                .await
        };
        let response = match result {
            Ok(Some(response)) => response,
            Ok(None) => {
                if step.state == ModelStepState::Dispatched {
                    step = store
                        .uncertain_model(actor, command(authority, actor, &step.job)?)
                        .await?;
                }
                return self.view(&step.job, Some(&step));
            }
            Err(error) => {
                if step.state == ModelStepState::Dispatched {
                    // If fencing/clock checks fail the durable DISPATCHED row is
                    // already sufficient uncertainty evidence. Never resend.
                    let _ = store
                        .uncertain_model(actor, command(authority, actor, &step.job)?)
                        .await;
                }
                return Err(error);
            }
        };
        plan.check()?;
        evidence.check(specification)?;
        let now = timestamp(authority.now()?);
        let outcome = match model_codec::response_ast(&step.request, &response, &plan.registry) {
            Ok(_) => v1::job_outcome::Outcome::Success(v1::JobSuccess { outputs: vec![] }),
            Err(_) => v1::job_outcome::Outcome::InfrastructureFailure(v1::InfrastructureFailure {
                error: Some(v1::ServiceError {
                    category: v1::ErrorCategory::Dependency as i32,
                    code: "model_candidate_invalid".into(),
                    message: "Provider output violates the frozen AST contract".into(),
                    retryable: false,
                    details: vec![],
                }),
                attempt: step.job.attempt,
                failed_at: Some(now),
            }),
        };
        step = store
            .finish_model(
                actor,
                command(authority, actor, &step.job)?,
                response,
                v1::JobOutcome {
                    outcome: Some(outcome),
                },
            )
            .await?;
        self.view(&step.job, Some(&step))
    }

    pub(crate) fn view(
        &self,
        job: &v1::JobRecord,
        step: Option<&ModelStep>,
    ) -> StoreResult<wire::DiscoveryStepView> {
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery specification"))?;
        let plan = self.plan(specification)?;
        if let Some(step) = step {
            if step.job != *job {
                return Err(StoreError::Corrupt("model view revision"));
            }
            check_request(plan, step)?;
        }
        let candidate = if let Some(step) = step.filter(|step| {
            step.state == ModelStepState::Completed && job.state == v1::JobState::Succeeded as i32
        }) {
            let ast = model_codec::response_ast(
                &step.request,
                step.response
                    .as_ref()
                    .ok_or(StoreError::Corrupt("model response"))?,
                &plan.registry,
            )?;
            Some(wire::DiscoveryCandidate {
                expression_id: ast.expression_id,
                canonicalization_profile: ast.canonicalization_profile,
                canonical_json: ast.canonical_json,
            })
        } else {
            None
        };
        let state = match step.map(|step| step.state) {
            None => wire::DiscoveryStepState::Unspecified,
            Some(ModelStepState::Reserved) => wire::DiscoveryStepState::Reserved,
            Some(ModelStepState::Dispatched) => wire::DiscoveryStepState::Dispatched,
            Some(ModelStepState::Ambiguous) => wire::DiscoveryStepState::Ambiguous,
            Some(ModelStepState::Completed) => wire::DiscoveryStepState::Completed,
        };
        Ok(wire::DiscoveryStepView {
            job: Some(wire::DiscoveryJobHandle {
                job_id: specification.job_id.clone(),
                status: job.state,
                revision: job.revision,
                submitted_at: specification.submitted_at,
                updated_at: job.updated_at,
            }),
            state: state as i32,
            candidate,
            reserved_cost: step.map(|step| v1::Money {
                currency_code: "USD".into(),
                amount: Some(v1::ExactDecimal {
                    value: usd(step.reserved_nano_usd),
                }),
            }),
            reserved_input_tokens: step.map_or(0, |step| step.reserved_input),
            reserved_output_tokens: step.map_or(0, |step| step.reserved_output),
        })
    }
}

fn check_request(plan: &super::plan::FrozenPlan, step: &ModelStep) -> StoreResult<()> {
    let invocation = step
        .request
        .invocation
        .as_ref()
        .ok_or(StoreError::Corrupt("model invocation"))?;
    let mut expected = plan.invocation.clone();
    expected.request_id = invocation.request_id.clone();
    if invocation != &expected {
        return Err(StoreError::Corrupt("frozen invocation changed"));
    }
    Ok(())
}

pub(super) fn context(
    authority: &RuntimeAuthority,
    actor: &v1::Actor,
    job_id: &str,
) -> StoreResult<v1::CommandContext> {
    let request_id = Uuid::new_v4().to_string();
    Ok(v1::CommandContext {
        request_id: Some(v1::RequestId {
            value: request_id.clone(),
        }),
        correlation_id: Some(v1::CorrelationId {
            value: job_id.into(),
        }),
        causation_id: Some(v1::CausationId {
            value: job_id.into(),
        }),
        idempotency_key: Some(v1::IdempotencyKey { value: request_id }),
        actor: Some(actor.clone()),
        requested_at: Some(timestamp(authority.now()?)),
    })
}

fn command(
    authority: &RuntimeAuthority,
    actor: &v1::Actor,
    job: &v1::JobRecord,
) -> StoreResult<ModelStepCommand> {
    let id = job
        .specification
        .as_ref()
        .and_then(|spec| spec.job_id.as_ref())
        .ok_or(StoreError::Corrupt("model job ID"))?;
    Ok(ModelStepCommand {
        context: Some(context(authority, actor, &id.value)?),
        job_id: Some(id.clone()),
        lease_id: job
            .active_lease
            .as_ref()
            .and_then(|lease| lease.lease_id.clone()),
        expected_revision: job.revision,
    })
}

fn remaining(authority: &RuntimeAuthority, step: &ModelStep, send: bool) -> StoreResult<Duration> {
    let expires = step
        .job
        .active_lease
        .as_ref()
        .and_then(|lease| lease.expires_at)
        .ok_or(StoreError::LeaseFenced)?;
    let expiry = expires
        .seconds
        .checked_mul(1000)
        .and_then(|value| value.checked_add(i64::from(expires.nanos) / 1_000_000))
        .ok_or(StoreError::Corrupt("model lease time"))?;
    let millis = expiry - authority.now()? - 1_000;
    if millis <= 0 {
        return Err(StoreError::LeaseFenced);
    }
    let invocation_limit = model_duration(
        step.request
            .invocation
            .as_ref()
            .and_then(|invocation| invocation.budget.as_ref())
            .and_then(|budget| budget.maximum_wall_time.as_ref()),
    )?;
    // Lookup has a five-second ceiling and must also respect deployments whose
    // accepted invocation budget is smaller. Recovery never extends that budget.
    let limit = if send {
        invocation_limit
    } else {
        invocation_limit.min(5_000)
    };
    Ok(Duration::from_millis(millis.min(limit) as u64))
}

fn timestamp(millis: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: millis.div_euclid(1000),
        nanos: (millis.rem_euclid(1000) * 1_000_000) as i32,
    }
}

fn usd(value: u64) -> String {
    if value.is_multiple_of(1_000_000_000) {
        return (value / 1_000_000_000).to_string();
    }
    format!("{}.{:09}", value / 1_000_000_000, value % 1_000_000_000)
        .trim_end_matches('0')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicI64};

    use super::*;
    use crate::runtime::{Identity, Role};
    use crate::test_support::{FixtureClock, NOW};

    fn authority() -> RuntimeAuthority {
        RuntimeAuthority::new(
            vec![Identity {
                actor_id: "agent.discovery".into(),
                subject: "agent:discovery".into(),
                display_name: "Discovery".into(),
                role: Role::Discovery,
                certificate_sha256: vec![format!("sha256:{}", "01".repeat(32))],
                not_before_ms: NOW - 1_000,
                expires_at_ms: NOW + 120_000,
                run_ids: vec!["run.discovery".into()],
            }],
            vec![],
            Arc::new(FixtureClock(AtomicI64::new(NOW))),
        )
        .unwrap()
    }

    fn step(wall_seconds: i64, lease_millis: i64) -> ModelStep {
        ModelStep {
            job: v1::JobRecord {
                active_lease: Some(v1::JobLease {
                    expires_at: Some(timestamp(NOW + lease_millis)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            request: InvokeModelRequest {
                invocation: Some(v1::ModelInvocation {
                    budget: Some(v1::InvocationBudget {
                        maximum_wall_time: Some(prost_types::Duration {
                            seconds: wall_seconds,
                            nanos: 0,
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            request_sha256: [0; 32],
            state: ModelStepState::Dispatched,
            response: None,
            reserved_input: 1,
            reserved_output: 1,
            reserved_nano_usd: 1,
        }
    }

    #[test]
    fn lookup_budget() {
        assert_eq!(
            remaining(&authority(), &step(2, 30_000), false).unwrap(),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn lookup_ceiling() {
        assert_eq!(
            remaining(&authority(), &step(20, 30_000), false).unwrap(),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn lease_deadline() {
        assert_eq!(
            remaining(&authority(), &step(20, 3_000), false).unwrap(),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn lease_expiry() {
        // The final second is reserved for committing evidence, not network I/O.
        assert!(matches!(
            remaining(&authority(), &step(20, 1_000), false),
            Err(StoreError::LeaseFenced)
        ));
    }
}
