use loop_protocol::wire::{runs::v1 as wire, v1};
use tonic::{Request, Response, Status};

use super::discovery::{check_deadline, validate_context, validate_deadline};
use super::{RuntimeService, status};
use crate::runtime::{Role, authority::Principal};
use crate::store::StoreError;

mod workflow;

#[tonic::async_trait]
impl wire::run_service_server::RunService for RuntimeService {
    async fn start_run(
        &self,
        request: Request<wire::StartRunRequest>,
    ) -> Result<Response<wire::StartRunResponse>, Status> {
        let principal = self.run_owner(&request, request.get_ref().context.as_ref())?;
        let catalog = self
            .runs
            .as_ref()
            .ok_or_else(|| status(StoreError::AdmissionDenied))?;
        let reference = request
            .get_ref()
            .plan
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("run plan")))?;
        let specification = catalog.resolve(reference).map_err(status)?;
        check_version(request.get_ref().workflow_version, specification).map_err(status)?;
        self.authority
            .verify_run(&principal.actor, specification)
            .map_err(status)?;
        let mut run = if specification.numerical.is_some() {
            let (resolver, frozen) = self.frozen_numerical(specification).await.map_err(status)?;
            let mut prepared =
                crate::runtime::workflow::PreparedRun::new(frozen).map_err(status)?;
            if let Some(proof) = self
                .initial_feedback(specification, &resolver)
                .await
                .map_err(status)?
            {
                prepared = prepared.with_adaptive(proof).map_err(status)?;
            }
            self.store
                .start_numerical(
                    &principal.actor,
                    request.get_ref(),
                    specification,
                    &prepared,
                )
                .await
                .map_err(status)?
        } else {
            self.store
                .start_run(&principal.actor, request.get_ref(), specification)
                .await
                .map_err(status)?
        };
        run.plan_verified = true;
        Ok(Response::new(wire::StartRunResponse { run: Some(run) }))
    }

    async fn get_run(
        &self,
        request: Request<wire::GetRunRequest>,
    ) -> Result<Response<wire::GetRunResponse>, Status> {
        let principal = self.run_owner(&request, request.get_ref().context.as_ref())?;
        let run_id = request
            .get_ref()
            .run_id
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("run ID")))?;
        self.authority
            .run_lookup(&principal.actor, &run_id.value)
            .map_err(status)?;
        let snapshot = self
            .store
            .read_run(&principal.actor, &run_id.value)
            .await
            .map_err(hidden_status)?;
        check_version(request.get_ref().workflow_version, &snapshot.specification)
            .map_err(status)?;
        let mut run = snapshot.view;
        if self
            .runs
            .as_ref()
            .is_some_and(|catalog| catalog.contains(snapshot.specification.plan.as_ref()))
        {
            self.authority
                .verify_run(&principal.actor, &snapshot.specification)
                .map_err(status)?;
            run.plan_verified = true;
        } else {
            run.plan_verified = false;
        }
        Ok(Response::new(wire::GetRunResponse { run: Some(run) }))
    }

    async fn step_run(
        &self,
        request: Request<wire::StepRunRequest>,
    ) -> Result<Response<wire::StepRunResponse>, Status> {
        let input = request.get_ref();
        let principal = if matches!(input.workflow_version, 2 | 3) {
            check_deadline(&request, 240_000_000_000)?;
            self.owner_context(&request, input.context.as_ref())?
        } else {
            self.run_owner(&request, input.context.as_ref())?
        };
        let run_id = input
            .run_id
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("run ID")))?;
        self.authority
            .run_lookup(&principal.actor, &run_id.value)
            .map_err(status)?;
        let snapshot = self
            .store
            .read_run(&principal.actor, &run_id.value)
            .await
            .map_err(hidden_status)?;
        check_version(input.workflow_version, &snapshot.specification).map_err(status)?;
        if let Some(mut run) = self
            .store
            .replay_step(&principal.actor, input)
            .await
            .map_err(status)?
        {
            run.plan_verified = false;
            if self
                .runs
                .as_ref()
                .is_some_and(|catalog| catalog.contains(snapshot.specification.plan.as_ref()))
            {
                self.authority
                    .verify_run(&principal.actor, &snapshot.specification)
                    .map_err(status)?;
                run.plan_verified = true;
            }
            return Ok(Response::new(wire::StepRunResponse { run: Some(run) }));
        }
        if snapshot.view.revision != input.expected_revision {
            return Err(status(StoreError::RevisionConflict));
        }
        self.authority
            .verify_run(&principal.actor, &snapshot.specification)
            .map_err(status)?;
        if snapshot.specification.numerical.is_some() {
            let mut run = Box::pin(self.step_numerical(&principal.actor, input, snapshot))
                .await
                .map_err(status)?;
            run.plan_verified = true;
            return Ok(Response::new(wire::StepRunResponse { run: Some(run) }));
        }
        let mut run = Box::pin(self.advance_legacy(&principal.actor, input, snapshot))
            .await
            .map_err(status)?;
        run.plan_verified = true;
        Ok(Response::new(wire::StepRunResponse { run: Some(run) }))
    }
}

impl RuntimeService {
    // Keep the legacy execution future out of the numerical RPC's poll frame.
    // This boundary retains its original authority and immutable replay rules.
    async fn advance_legacy(
        &self,
        owner: &v1::Actor,
        input: &wire::StepRunRequest,
        snapshot: crate::store::RunSnapshot,
    ) -> crate::store::StoreResult<wire::RunView> {
        let execution = RunExecution::new(snapshot, self.authority.now()?)?;
        let discoverer = self
            .discoverer
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        if execution.active {
            discoverer
                .execute(
                    &self.store,
                    &self.authority,
                    &execution.actor,
                    execution.job.clone(),
                    execution.job.revision,
                )
                .await?;
        }
        self.authority.verify_run(owner, &execution.specification)?;
        // The child lease fences model calls; the parent CAS fences advancement.
        // If this process dies here, the next request observes the committed child
        // and never replaces or redispatches it.
        self.store
            .advance_run(owner, input, &execution.specification)
            .await
    }
}

impl RuntimeService {
    fn run_owner<T>(
        &self,
        request: &Request<T>,
        context: Option<&v1::CommandContext>,
    ) -> Result<Principal, Status> {
        validate_deadline(request)?;
        self.owner_context(request, context)
    }

    fn owner_context<T>(
        &self,
        request: &Request<T>,
        context: Option<&v1::CommandContext>,
    ) -> Result<Principal, Status> {
        let principal = self.authority.authenticate(request).map_err(status)?;
        if principal.identity.role != Role::Operator {
            return Err(status(StoreError::AdmissionDenied));
        }
        validate_context(
            context,
            &principal.actor,
            self.authority.now().map_err(status)?,
        )?;
        Ok(principal)
    }
}

// Nonserializable, handler-local child scope. The authenticated human is never
// reused as the Agent principal; the executor identity comes from the verified
// immutable specification and must equal the current child's submitted_by.
struct RunExecution {
    specification: wire::RunSpecification,
    actor: v1::Actor,
    job: v1::JobRecord,
    active: bool,
}

impl RunExecution {
    fn new(snapshot: crate::store::RunSnapshot, now: i64) -> crate::store::StoreResult<Self> {
        let actor = snapshot
            .specification
            .executor
            .clone()
            .ok_or(StoreError::Corrupt("run executor"))?;
        let job_spec = snapshot
            .current_job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("run child"))?;
        if job_spec.submitted_by.as_ref() != Some(&actor)
            || job_spec.run_id != snapshot.specification.run_id
        {
            return Err(StoreError::Corrupt("run execution binding"));
        }
        let deadline = snapshot
            .view
            .deadline
            .as_ref()
            .ok_or(StoreError::Corrupt("run deadline"))?;
        let deadline = deadline
            .seconds
            .checked_mul(1_000)
            .and_then(|seconds| seconds.checked_add(i64::from(deadline.nanos) / 1_000_000))
            .ok_or(StoreError::Corrupt("run deadline range"))?;
        Ok(Self {
            specification: snapshot.specification,
            actor,
            job: snapshot.current_job,
            active: snapshot.view.status == wire::RunStatus::Active as i32 && now < deadline,
        })
    }
}

fn hidden_status(error: StoreError) -> Status {
    status(match error {
        StoreError::NotFound => StoreError::AdmissionDenied,
        other => other,
    })
}

fn check_version(
    version: u32,
    specification: &wire::RunSpecification,
) -> crate::store::StoreResult<()> {
    loop_protocol::runs::validate_workflow(version)?;
    if matches!(version, 2 | 3) != specification.numerical.is_some()
        || (version == 3) != specification.feedback_policy.is_some()
    {
        return Err(StoreError::Invalid("run workflow version mismatch"));
    }
    Ok(())
}
