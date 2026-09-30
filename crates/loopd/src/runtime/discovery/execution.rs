use std::future::Future;
use std::time::Duration;

use loop_protocol::wire::{discovery::v1 as wire, provider::v1::InvokeModelRequest, v1};
use uuid::Uuid;

use super::DiscoveryExecutor;
use crate::runtime::{RuntimeAuthority, model_codec};
use crate::store::{
    ModelControl, ModelRetry, ModelStep, ModelStepCommand, ModelStepState, PgJobStore, StoreError,
    StoreResult, model_duration,
};

// Safe-attempt loops return a static terminal label separately from storage,
// clock and fencing errors. The caller retains the latest durable step in both
// cases; no helper can reset its counter or silently terminalize another lease.
enum Attempt<T> {
    Ready(T),
    Failed(&'static str),
}

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
        if job.state == v1::JobState::Paused as i32 {
            return Err(StoreError::InvalidTransition);
        }
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery specification"))?;
        let plan = self.plan(specification)?;
        let job_id = &specification
            .job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery job ID"))?
            .value;
        let history = store.model_history(actor, job_id).await?;
        check_history(plan, &history)?;
        if terminal(job.state) {
            return self.view(&job, &history);
        }
        if authority.now()? >= deadline(&job)? {
            let mut control = command(authority, actor, &job, 0)?;
            control.lease_id = None;
            let job = store
                .control_model(actor, control, ModelControl::Expire)
                .await?;
            return self.current_view(store, actor, &job).await;
        }
        crate::manifests::data::resolve(&self.data, &plan.data, specification, false).await?;
        let step = if let Some(step) = history.last() {
            if step.job.revision != expected_revision {
                return Err(StoreError::RevisionConflict);
            }
            // A second RPC cannot borrow another in-flight handler's lease.
            let mut takeover = command(authority, actor, &step.job, step.ordinal)?;
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
            let invocation = plan
                .tool_invocation
                .as_ref()
                .unwrap_or(&plan.invocation)
                .clone();
            let request = request(authority, actor, job_id, invocation)?;
            store
                .reserve_model(actor, command(authority, actor, &job, 0)?, request)
                .await?
        };
        Box::pin(self.drive(store, authority, actor, step)).await
    }

    pub(crate) async fn resume(
        &self,
        store: &PgJobStore,
        authority: &RuntimeAuthority,
        actor: &v1::Actor,
        job: v1::JobRecord,
        control: ModelStepCommand,
    ) -> StoreResult<wire::DiscoveryStepView> {
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery specification"))?;
        let plan = self.plan(specification)?;
        let id = specification
            .job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery job ID"))?;
        let history = store.model_history(actor, &id.value).await?;
        check_history(plan, &history)?;
        let expired = authority.now()? >= deadline(&job)?;
        if !expired {
            crate::manifests::data::resolve(&self.data, &plan.data, specification, false).await?;
        }
        let initial = if history.is_empty() && !expired {
            Some(request(
                authority,
                actor,
                &id.value,
                plan.tool_invocation
                    .as_ref()
                    .unwrap_or(&plan.invocation)
                    .clone(),
            )?)
        } else {
            None
        };
        let claim = store.resume_model(actor, control, initial).await?;
        if !claim.execute {
            return self.current_view(store, actor, &claim.job).await;
        }
        Box::pin(
            self.drive(
                store,
                authority,
                actor,
                claim
                    .step
                    .ok_or(StoreError::Corrupt("resumed model step"))?,
            ),
        )
        .await
    }

    pub(crate) async fn reconcile(
        &self,
        store: &PgJobStore,
        authority: &RuntimeAuthority,
        actor: &v1::Actor,
        job: v1::JobRecord,
        control: ModelStepCommand,
    ) -> StoreResult<wire::DiscoveryStepView> {
        if let Some(replayed) = store.reconcile_replay(actor, control.clone()).await? {
            return self.current_view(store, actor, &replayed).await;
        }
        if !matches!(
            v1::JobState::try_from(job.state),
            Ok(v1::JobState::Paused
                | v1::JobState::Cancelled
                | v1::JobState::BudgetExhausted
                | v1::JobState::InfrastructureFailed)
        ) {
            return Err(StoreError::InvalidTransition);
        }
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery specification"))?;
        let plan = self.plan(specification)?;
        let id = specification
            .job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery job ID"))?;
        let history = store.model_history(actor, &id.value).await?;
        check_history(plan, &history)?;
        let Some(step) = history.last() else {
            if control.expected_revision != job.revision {
                return Err(StoreError::RevisionConflict);
            }
            return self.view(&job, &history);
        };
        if step.job.revision != control.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if step.state == ModelStepState::Reserved || step.state == ModelStepState::Completed {
            return self.view(&step.job, &history);
        }
        // Evidence lookup has a fresh caller deadline; an elapsed execution
        // budget never grants a lease or a new paid invocation here.
        let response = self
            .provider
            .lookup(
                step,
                context(authority, actor, &id.value)?,
                Duration::from_secs(5),
            )
            .await?;
        if let Some(response) = response {
            let mut control = control;
            control.ordinal = step.ordinal;
            let reconciled = store.reconcile_model(actor, control, response).await?;
            return self.current_view(store, actor, &reconciled.job).await;
        }
        self.view(&step.job, &history)
    }

    async fn drive(
        &self,
        store: &PgJobStore,
        authority: &RuntimeAuthority,
        actor: &v1::Actor,
        mut step: ModelStep,
    ) -> StoreResult<wire::DiscoveryStepView> {
        let specification = step
            .job
            .specification
            .clone()
            .ok_or(StoreError::Corrupt("discovery specification"))?;
        let plan = self.plan(&specification)?;
        let evidence = guarded(
            store,
            authority,
            actor,
            &step,
            crate::manifests::data::resolve(&self.data, &plan.data, &specification, false),
        )
        .await?;
        let job_id = &specification
            .job_id
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery job ID"))?
            .value;
        // The immutable plan permits at most two model turns, never an unbounded
        // caller-driven conversation. A committed tool result is reused verbatim.
        loop {
            plan.check()?;
            evidence.check(&specification)?;
            if step.state == ModelStepState::Completed {
                if plan.tool_invocation.is_none() || step.ordinal != 0 {
                    let response = step
                        .response
                        .clone()
                        .ok_or(StoreError::Corrupt("model response"))?;
                    return self
                        .complete(store, authority, actor, &step, response)
                        .await;
                }
                if step.tool_result.is_none() {
                    let result =
                        match Box::pin(self.tool_step(store, authority, actor, &mut step)).await? {
                            Attempt::Ready(result) => result,
                            Attempt::Failed(code) => {
                                return self.fail(store, authority, actor, &step, code).await;
                            }
                        };
                    step = store
                        .record_tool(actor, command(authority, actor, &step.job, 0)?, result)
                        .await?;
                }
                let invocation = continuation(plan, &step)?;
                let request = request(authority, actor, job_id, invocation)?;
                step = store
                    .reserve_model(actor, command(authority, actor, &step.job, 1)?, request)
                    .await?;
            }
            let send = if step.state == ModelStepState::Reserved {
                let dispatch = store
                    .dispatch_model(actor, command(authority, actor, &step.job, step.ordinal)?)
                    .await?;
                step = dispatch.step;
                dispatch.send
            } else {
                false
            };
            let response = if send {
                let result = guarded(
                    store,
                    authority,
                    actor,
                    &step,
                    self.provider
                        .invoke(step.request.clone(), remaining(authority, &step, true)?),
                )
                .await;
                match result {
                    Ok(response) => Some(response),
                    Err(StoreError::Unavailable(_)) => {
                        step = store
                            .uncertain_model(
                                actor,
                                command(authority, actor, &step.job, step.ordinal)?,
                            )
                            .await?;
                        None
                    }
                    Err(StoreError::AdmissionDenied | StoreError::Corrupt(_)) => {
                        return self
                            .fail(store, authority, actor, &step, "model_contract_invalid")
                            .await;
                    }
                    Err(error) => return Err(error),
                }
            } else {
                None
            };
            let response = if let Some(response) = response {
                response
            } else {
                match Box::pin(self.lookup_step(store, authority, actor, &mut step)).await? {
                    Attempt::Ready(response) => response,
                    Attempt::Failed(code) => {
                        return self.fail(store, authority, actor, &step, code).await;
                    }
                }
            };
            if step.check_response(&response).is_err() {
                return self
                    .fail(store, authority, actor, &step, "model_contract_invalid")
                    .await;
            }
            plan.check()?;
            evidence.check(&specification)?;
            let intermediate = plan.tool_invocation.is_some() && step.ordinal == 0;
            if intermediate && model_codec::response_call(&step.request, &response).is_ok() {
                step = store
                    .finish_call(
                        actor,
                        command(authority, actor, &step.job, step.ordinal)?,
                        response,
                    )
                    .await?;
                continue;
            }
            return self
                .complete(store, authority, actor, &step, response)
                .await;
        }
    }

    async fn tool_step(
        &self,
        store: &PgJobStore,
        authority: &RuntimeAuthority,
        actor: &v1::Actor,
        step: &mut ModelStep,
    ) -> StoreResult<Attempt<v1::ToolResultContent>> {
        let specification = step
            .job
            .specification
            .clone()
            .ok_or(StoreError::Corrupt("discovery specification"))?;
        let plan = self.plan(&specification)?;
        let Ok(call) = model_codec::response_call(
            &step.request,
            step.response
                .as_ref()
                .ok_or(StoreError::Corrupt("tool response"))?,
        ) else {
            return Ok(Attempt::Failed("tool_contract_invalid"));
        };
        loop {
            if step.tool_attempts >= 3 {
                return Ok(Attempt::Failed("retry_exhausted"));
            }
            retry_wait(store, authority, actor, step).await?;
            *step = store
                .retry_model(
                    actor,
                    command(authority, actor, &step.job, step.ordinal)?,
                    ModelRetry::Tool,
                )
                .await?;
            let result = tokio::time::timeout(
                Duration::from_millis(lease_millis(authority, step)?.min(30_000) as u64),
                guarded(
                    store,
                    authority,
                    actor,
                    step,
                    super::tools::describe(
                        &self.data,
                        &plan.data,
                        &specification,
                        &plan.registry,
                        &call,
                    ),
                ),
            )
            .await
            .map_err(|_| StoreError::Unavailable("research tool deadline"))
            .and_then(|result| result);
            match result {
                Ok(result) => return Ok(Attempt::Ready(result)),
                Err(StoreError::Unavailable(_)) => continue,
                Err(
                    error @ (StoreError::LeaseFenced
                    | StoreError::RevisionConflict
                    | StoreError::ClockRegression),
                ) => return Err(error),
                Err(_) => return Ok(Attempt::Failed("tool_contract_invalid")),
            }
        }
    }

    async fn lookup_step(
        &self,
        store: &PgJobStore,
        authority: &RuntimeAuthority,
        actor: &v1::Actor,
        step: &mut ModelStep,
    ) -> StoreResult<Attempt<v1::ModelResponse>> {
        let job_id = step
            .job
            .specification
            .as_ref()
            .and_then(|spec| spec.job_id.as_ref())
            .ok_or(StoreError::Corrupt("discovery job ID"))?
            .value
            .clone();
        loop {
            if step.lookup_attempts >= 3 {
                return Ok(Attempt::Failed("retry_exhausted"));
            }
            retry_wait(store, authority, actor, step).await?;
            *step = store
                .retry_model(
                    actor,
                    command(authority, actor, &step.job, step.ordinal)?,
                    ModelRetry::Lookup,
                )
                .await?;
            let result = guarded(
                store,
                authority,
                actor,
                step,
                self.provider.lookup(
                    step,
                    context(authority, actor, &job_id)?,
                    remaining(authority, step, false)?,
                ),
            )
            .await;
            match result {
                Ok(Some(response)) => return Ok(Attempt::Ready(response)),
                Ok(None) | Err(StoreError::Unavailable(_)) => continue,
                Err(StoreError::AdmissionDenied | StoreError::Corrupt(_)) => {
                    return Ok(Attempt::Failed("model_contract_invalid"));
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn complete(
        &self,
        store: &PgJobStore,
        authority: &RuntimeAuthority,
        actor: &v1::Actor,
        step: &ModelStep,
        response: v1::ModelResponse,
    ) -> StoreResult<wire::DiscoveryStepView> {
        let plan = self.plan(
            step.job
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("discovery specification"))?,
        )?;
        let outcome = if model_codec::response_ast(&step.request, &response, &plan.registry).is_ok()
        {
            v1::job_outcome::Outcome::Success(v1::JobSuccess { outputs: vec![] })
        } else {
            v1::job_outcome::Outcome::InfrastructureFailure(v1::InfrastructureFailure {
                error: Some(v1::ServiceError {
                    category: v1::ErrorCategory::Dependency as i32,
                    code: "model_candidate_invalid".into(),
                    message: "Provider output violates the frozen conversation contract".into(),
                    retryable: false,
                    details: vec![],
                }),
                attempt: step.job.attempt,
                failed_at: Some(timestamp(authority.now()?)),
            })
        };
        store
            .finish_model(
                actor,
                command(authority, actor, &step.job, step.ordinal)?,
                response,
                v1::JobOutcome {
                    outcome: Some(outcome),
                },
            )
            .await?;
        self.current_view(store, actor, &step.job).await
    }

    async fn fail(
        &self,
        store: &PgJobStore,
        authority: &RuntimeAuthority,
        actor: &v1::Actor,
        step: &ModelStep,
        code: &'static str,
    ) -> StoreResult<wire::DiscoveryStepView> {
        let job = store
            .fail_model(
                actor,
                command(authority, actor, &step.job, step.ordinal)?,
                code,
            )
            .await?;
        self.current_view(store, actor, &job).await
    }

    async fn current_view(
        &self,
        store: &PgJobStore,
        actor: &v1::Actor,
        job: &v1::JobRecord,
    ) -> StoreResult<wire::DiscoveryStepView> {
        let id = &job
            .specification
            .as_ref()
            .and_then(|spec| spec.job_id.as_ref())
            .ok_or(StoreError::Corrupt("discovery job ID"))?
            .value;
        let history = store.model_history(actor, id).await?;
        let job = history.last().map_or(job, |step| &step.job);
        self.view(job, &history)
    }

    pub(crate) fn view(
        &self,
        job: &v1::JobRecord,
        history: &[ModelStep],
    ) -> StoreResult<wire::DiscoveryStepView> {
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery specification"))?;
        let plan = self.plan(specification)?;
        check_history(plan, history)?;
        let step = history.last();
        if let Some(step) = step
            && step.job != *job
        {
            return Err(StoreError::Corrupt("model view revision"));
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
            candidate,
            reserved_cost: step.map(|_| v1::Money {
                currency_code: "USD".into(),
                amount: Some(v1::ExactDecimal { value: usd(cost) }),
            }),
            reserved_input_tokens: input,
            reserved_output_tokens: output,
        })
    }
}

fn check_history(plan: &super::plan::FrozenPlan, history: &[ModelStep]) -> StoreResult<()> {
    if history.len() > if plan.tool_invocation.is_some() { 2 } else { 1 } {
        return Err(StoreError::Corrupt("model history bounds"));
    }
    for (index, step) in history.iter().enumerate() {
        if step.ordinal as usize != index {
            return Err(StoreError::Corrupt("model history order"));
        }
        let invocation = step
            .request
            .invocation
            .as_ref()
            .ok_or(StoreError::Corrupt("model invocation"))?;
        let mut expected = if index == 0 {
            plan.tool_invocation
                .as_ref()
                .unwrap_or(&plan.invocation)
                .clone()
        } else {
            continuation(plan, &history[0])?
        };
        expected.request_id = invocation.request_id.clone();
        if invocation != &expected {
            return Err(StoreError::Corrupt("frozen invocation changed"));
        }
    }
    Ok(())
}

fn terminal(state: i32) -> bool {
    matches!(
        v1::JobState::try_from(state),
        Ok(v1::JobState::Succeeded
            | v1::JobState::FactorRejected
            | v1::JobState::InfrastructureFailed
            | v1::JobState::Cancelled
            | v1::JobState::BudgetExhausted)
    )
}

fn deadline(job: &v1::JobRecord) -> StoreResult<i64> {
    let spec = job
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("discovery specification"))?;
    let Some(v1::job_specification::Input::Discovery(input)) = &spec.input else {
        return Err(StoreError::AdmissionDenied);
    };
    let submitted = crate::store::timestamp_millis(
        spec.submitted_at
            .as_ref()
            .ok_or(StoreError::Corrupt("discovery submitted time"))?,
        false,
    )?;
    submitted
        .checked_add(model_duration(
            input
                .budget
                .as_ref()
                .and_then(|budget| budget.maximum_wall_time.as_ref()),
        )?)
        .ok_or(StoreError::Corrupt("discovery deadline"))
}

async fn check_running(
    store: &PgJobStore,
    authority: &RuntimeAuthority,
    actor: &v1::Actor,
    step: &ModelStep,
) -> StoreResult<()> {
    let id = step
        .job
        .specification
        .as_ref()
        .and_then(|spec| spec.job_id.as_ref())
        .ok_or(StoreError::Corrupt("discovery job ID"))?;
    let job = tokio::time::timeout(
        Duration::from_secs(1),
        store.runtime_job(actor, &id.value, "loop.discovery.read_control"),
    )
    .await
    .map_err(|_| StoreError::Unavailable("discovery control check"))??;
    if job.revision != step.job.revision
        || job.active_lease != step.job.active_lease
        || job.state != v1::JobState::Running as i32
    {
        return Err(StoreError::LeaseFenced);
    }
    if authority.now()? >= deadline(&job)? {
        let mut expiry = command(authority, actor, &job, 0)?;
        expiry.lease_id = None;
        store
            .control_model(actor, expiry, ModelControl::Expire)
            .await?;
        return Err(StoreError::LeaseFenced);
    }
    lease_millis(authority, step)?;
    Ok(())
}

async fn guarded<T>(
    store: &PgJobStore,
    authority: &RuntimeAuthority,
    actor: &v1::Actor,
    step: &ModelStep,
    operation: impl Future<Output = StoreResult<T>>,
) -> StoreResult<T> {
    Box::pin(check_running(store, authority, actor, step)).await?;
    tokio::pin!(operation);
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        tokio::select! {
            result = &mut operation => return result,
            _ = interval.tick() => Box::pin(check_running(store, authority, actor, step)).await?,
        }
    }
}

async fn retry_wait(
    store: &PgJobStore,
    authority: &RuntimeAuthority,
    actor: &v1::Actor,
    step: &ModelStep,
) -> StoreResult<()> {
    let delay = step.retry_after_ms.saturating_sub(authority.now()?).max(0);
    if delay > 250 {
        return Err(StoreError::ClockRegression);
    }
    guarded(store, authority, actor, step, async {
        tokio::time::sleep(Duration::from_millis(delay as u64)).await;
        Ok(())
    })
    .await
}

fn continuation(
    plan: &super::plan::FrozenPlan,
    step: &ModelStep,
) -> StoreResult<v1::ModelInvocation> {
    let response = step
        .response
        .as_ref()
        .ok_or(StoreError::Corrupt("missing tool response"))?;
    let call = model_codec::response_call(&step.request, response)?;
    let result = step
        .tool_result
        .as_ref()
        .ok_or(StoreError::Corrupt("missing tool result"))?;
    super::tools::validate_result(result)?;
    if step.ordinal != 0 || result.tool_call_id != call.tool_call_id {
        return Err(StoreError::Corrupt("conversation tool binding"));
    }
    let mut invocation = plan.invocation.clone();
    invocation.messages.extend([
        v1::ModelMessage {
            role: v1::ModelRole::Assistant as i32,
            content: response.content.clone(),
        },
        v1::ModelMessage {
            role: v1::ModelRole::Tool as i32,
            content: vec![v1::ContentBlock {
                content: Some(v1::content_block::Content::ToolResult(result.clone())),
            }],
        },
    ]);
    model_codec::validate_context(&invocation.messages)?;
    Ok(invocation)
}

fn request(
    authority: &RuntimeAuthority,
    actor: &v1::Actor,
    job: &str,
    mut invocation: v1::ModelInvocation,
) -> StoreResult<InvokeModelRequest> {
    let context = context(authority, actor, job)?;
    invocation.request_id = context.request_id.clone();
    Ok(InvokeModelRequest {
        context: Some(context),
        invocation: Some(invocation),
    })
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
    ordinal: u32,
) -> StoreResult<ModelStepCommand> {
    let id = job
        .specification
        .as_ref()
        .and_then(|spec| spec.job_id.as_ref())
        .ok_or(StoreError::Corrupt("model job ID"))?;
    Ok(ModelStepCommand {
        ordinal,
        context: Some(context(authority, actor, &id.value)?),
        job_id: Some(id.clone()),
        lease_id: job
            .active_lease
            .as_ref()
            .and_then(|lease| lease.lease_id.clone()),
        expected_revision: job.revision,
    })
}

fn lease_millis(authority: &RuntimeAuthority, step: &ModelStep) -> StoreResult<i64> {
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
    let millis = expiry
        .checked_sub(authority.now()?)
        .and_then(|value| value.checked_sub(1_000))
        .ok_or(StoreError::Corrupt("model lease time"))?;
    if millis <= 0 {
        return Err(StoreError::LeaseFenced);
    }
    Ok(millis)
}

fn remaining(authority: &RuntimeAuthority, step: &ModelStep, send: bool) -> StoreResult<Duration> {
    let millis = lease_millis(authority, step)?;
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
            lookup_attempts: 0,
            tool_attempts: 0,
            retry_after_ms: 0,
            ordinal: 0,
            tool_result: None,
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
