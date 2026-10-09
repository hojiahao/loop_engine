use loop_protocol::wire::discovery::v1::{
    self as wire, discovery_service_server::DiscoveryService,
};
use tonic::{Request, Response, Status};

use super::{RuntimeService, status};
use crate::runtime::Role;
use crate::store::{
    JobRepository, ModelControl, ModelStepCommand, RoleCommand, RoleJobHandle, StoreError,
};
use loop_protocol::wire::v1;

#[tonic::async_trait]
impl DiscoveryService for RuntimeService {
    async fn pause_discovery(
        &self,
        request: Request<wire::PauseDiscoveryRequest>,
    ) -> Result<Response<wire::PauseDiscoveryResponse>, Status> {
        let input = request.get_ref();
        let job = self
            .stop_discovery(
                &request,
                input.context.as_ref(),
                input.job_id.as_ref(),
                input.expected_revision,
                ModelControl::Pause,
            )
            .await?;
        Ok(Response::new(wire::PauseDiscoveryResponse {
            job: Some(job),
        }))
    }

    async fn cancel_discovery(
        &self,
        request: Request<wire::CancelDiscoveryRequest>,
    ) -> Result<Response<wire::CancelDiscoveryResponse>, Status> {
        let input = request.get_ref();
        let job = self
            .stop_discovery(
                &request,
                input.context.as_ref(),
                input.job_id.as_ref(),
                input.expected_revision,
                ModelControl::Cancel,
            )
            .await?;
        Ok(Response::new(wire::CancelDiscoveryResponse {
            job: Some(job),
        }))
    }

    async fn expire_discovery(
        &self,
        request: Request<wire::ExpireDiscoveryRequest>,
    ) -> Result<Response<wire::ExpireDiscoveryResponse>, Status> {
        let input = request.get_ref();
        let job = self
            .stop_discovery(
                &request,
                input.context.as_ref(),
                input.job_id.as_ref(),
                input.expected_revision,
                ModelControl::Expire,
            )
            .await?;
        Ok(Response::new(wire::ExpireDiscoveryResponse {
            job: Some(job),
        }))
    }

    async fn resume_discovery(
        &self,
        request: Request<wire::ResumeDiscoveryRequest>,
    ) -> Result<Response<wire::ResumeDiscoveryResponse>, Status> {
        validate_deadline(&request)?;
        let input = request.get_ref();
        let (principal, job) = self
            .job(
                &request,
                input.job_id.as_ref().map(|id| id.value.as_str()),
                "loop.model.resume",
                false,
            )
            .await
            .map_err(status)?;
        validate_context(
            input.context.as_ref(),
            &principal.actor,
            self.authority.now().map_err(status)?,
        )?;
        let executor = self
            .discoverer
            .as_ref()
            .ok_or_else(|| status(StoreError::AdmissionDenied))?;
        let command = ModelStepCommand {
            context: input.context.clone(),
            job_id: input.job_id.clone(),
            expected_revision: input.expected_revision,
            lease_id: None,
            ordinal: 0,
        };
        let step = executor
            .resume(&self.store, &self.authority, &principal.actor, job, command)
            .await
            .map_err(status)?;
        Ok(Response::new(wire::ResumeDiscoveryResponse {
            step: Some(step),
        }))
    }

    async fn reconcile_discovery(
        &self,
        request: Request<wire::ReconcileDiscoveryRequest>,
    ) -> Result<Response<wire::ReconcileDiscoveryResponse>, Status> {
        validate_deadline(&request)?;
        let input = request.get_ref();
        let (principal, job) = self
            .job(
                &request,
                input.job_id.as_ref().map(|id| id.value.as_str()),
                "loop.model.reconcile",
                false,
            )
            .await
            .map_err(status)?;
        validate_context(
            input.context.as_ref(),
            &principal.actor,
            self.authority.now().map_err(status)?,
        )?;
        let executor = self
            .discoverer
            .as_ref()
            .ok_or_else(|| status(StoreError::AdmissionDenied))?;
        let command = ModelStepCommand {
            context: input.context.clone(),
            job_id: input.job_id.clone(),
            expected_revision: input.expected_revision,
            lease_id: None,
            ordinal: 0,
        };
        let mut step = executor
            .reconcile(&self.store, &self.authority, &principal.actor, job, command)
            .await
            .map_err(status)?;
        // A concurrent Resume may complete before the final evidence read.
        // This lookup-only RPC never exposes a candidate, even in that race.
        step.candidate = None;
        Ok(Response::new(wire::ReconcileDiscoveryResponse {
            step: Some(step),
        }))
    }

    async fn start_discovery(
        &self,
        request: Request<wire::StartDiscoveryRequest>,
    ) -> Result<Response<wire::StartDiscoveryResponse>, Status> {
        validate_deadline(&request)?;
        let principal = self.authority.authenticate(&request).map_err(status)?;
        validate_context(
            request.get_ref().context.as_ref(),
            &principal.actor,
            self.authority.now().map_err(status)?,
        )?;
        let executor = self
            .discoverer
            .as_ref()
            .ok_or_else(|| status(StoreError::AdmissionDenied))?;
        if principal.identity.role != Role::Discovery {
            return Err(status(StoreError::AdmissionDenied));
        }
        let input = request
            .get_ref()
            .discovery
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("discovery input")))?;
        let metadata = executor
            .submission(&principal.actor, input)
            .map_err(status)?;
        let result = self
            .store
            .submit_role(
                &principal.actor,
                RoleCommand::Discovery(request.into_inner()),
                metadata,
            )
            .await
            .map_err(status)?;
        let RoleJobHandle::Discovery(job) = result.handle else {
            return Err(status(StoreError::Corrupt("discovery receipt")));
        };
        Ok(Response::new(wire::StartDiscoveryResponse {
            job: Some(job),
        }))
    }

    async fn execute_discovery(
        &self,
        request: Request<wire::ExecuteDiscoveryRequest>,
    ) -> Result<Response<wire::ExecuteDiscoveryResponse>, Status> {
        validate_deadline(&request)?;
        let command = request.get_ref();
        let (principal, job) = self
            .job(
                &request,
                command.job_id.as_ref().map(|id| id.value.as_str()),
                "loop.model.read",
                false,
            )
            .await
            .map_err(status)?;
        validate_context(
            command.context.as_ref(),
            &principal.actor,
            self.authority.now().map_err(status)?,
        )?;
        let executor = self
            .discoverer
            .as_ref()
            .ok_or_else(|| status(StoreError::AdmissionDenied))?;
        let step = executor
            .execute(
                &self.store,
                &self.authority,
                &principal.actor,
                job,
                command.expected_revision,
            )
            .await
            .map_err(status)?;
        Ok(Response::new(wire::ExecuteDiscoveryResponse {
            step: Some(step),
        }))
    }

    async fn get_discovery(
        &self,
        request: Request<wire::GetDiscoveryRequest>,
    ) -> Result<Response<wire::GetDiscoveryResponse>, Status> {
        validate_deadline(&request)?;
        let command = request.get_ref();
        let (principal, _) = self
            .job(
                &request,
                command.job_id.as_ref().map(|id| id.value.as_str()),
                "loop.discovery.read_control",
                false,
            )
            .await
            .map_err(status)?;
        validate_context(
            command.context.as_ref(),
            &principal.actor,
            self.authority.now().map_err(status)?,
        )?;
        let id = &command
            .job_id
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("discovery job ID")))?
            .value;
        let (job, history) = self
            .store
            .discovery_status(&principal.actor, id)
            .await
            .map_err(status)?;
        let view = match &self.discoverer {
            Some(executor) => executor.view(&job, &history),
            None => crate::runtime::discovery::metadata(&job, &history),
        }
        .map_err(status)?;
        Ok(Response::new(wire::GetDiscoveryResponse {
            step: Some(view),
        }))
    }

    async fn list_discovery_events(
        &self,
        request: Request<wire::ListDiscoveryEventsRequest>,
    ) -> Result<Response<wire::ListDiscoveryEventsResponse>, Status> {
        validate_deadline(&request)?;
        let command = request.get_ref();
        let (principal, _) = self
            .job(
                &request,
                command.job_id.as_ref().map(|id| id.value.as_str()),
                "loop.discovery.read_control",
                false,
            )
            .await
            .map_err(status)?;
        validate_context(
            command.context.as_ref(),
            &principal.actor,
            self.authority.now().map_err(status)?,
        )?;
        let id = &command
            .job_id
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("discovery job ID")))?
            .value;
        let page = self
            .store
            .discovery_events(&principal.actor, id, command.after_sequence, command.limit)
            .await
            .map_err(status)?;
        Ok(Response::new(page))
    }
}

impl RuntimeService {
    async fn stop_discovery<T>(
        &self,
        request: &Request<T>,
        context: Option<&v1::CommandContext>,
        id: Option<&v1::JobId>,
        revision: u64,
        action: ModelControl,
    ) -> Result<wire::DiscoveryJobHandle, Status> {
        validate_deadline(request)?;
        let operation = match action {
            ModelControl::Pause => "loop.discovery.pause",
            ModelControl::Cancel => "loop.discovery.cancel",
            ModelControl::Expire => "loop.discovery.expire",
        };
        let (principal, _) = self
            .job(request, id.map(|id| id.value.as_str()), operation, false)
            .await
            .map_err(status)?;
        validate_context(
            context,
            &principal.actor,
            self.authority.now().map_err(status)?,
        )?;
        let job = self
            .store
            .control_model(
                &principal.actor,
                ModelStepCommand {
                    context: context.cloned(),
                    job_id: id.cloned(),
                    expected_revision: revision,
                    lease_id: None,
                    ordinal: 0,
                },
                action,
            )
            .await
            .map_err(status)?;
        let specification = job
            .specification
            .as_ref()
            .ok_or_else(|| status(StoreError::Corrupt("discovery specification")))?;
        Ok(wire::DiscoveryJobHandle {
            job_id: specification.job_id.clone(),
            status: job.state,
            revision: job.revision,
            submitted_at: specification.submitted_at,
            updated_at: job.updated_at,
        })
    }
}

pub(super) fn validate_deadline<T>(request: &Request<T>) -> Result<(), Status> {
    let invalid = || status(StoreError::Invalid("discovery caller deadline"));
    let value = request
        .metadata()
        .get("grpc-timeout")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(invalid)?;
    if !(2..=9).contains(&value.len()) || !value.is_ascii() {
        return Err(invalid());
    }
    let (digits, unit) = value.split_at(value.len() - 1);
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let amount = digits.parse::<u64>().map_err(|_| invalid())?;
    let nanos = match unit {
        "H" => 3_600_000_000_000,
        "M" => 60_000_000_000,
        "S" => 1_000_000_000,
        "m" => 1_000_000,
        "u" => 1_000,
        "n" => 1,
        _ => return Err(invalid()),
    };
    if amount
        .checked_mul(nanos)
        .is_none_or(|nanos| nanos == 0 || nanos > 120_000_000_000)
    {
        return Err(invalid());
    }
    // Tonic's transport deadline layer cancels the handler at this duration.
    // A cancellation after dispatch leaves conservative durable evidence.
    Ok(())
}

pub(super) fn validate_context(
    context: Option<&loop_protocol::wire::v1::CommandContext>,
    actor: &loop_protocol::wire::v1::Actor,
    now: i64,
) -> Result<(), Status> {
    let context = crate::store::validate_runtime_context(context, actor).map_err(status)?;
    let time = context
        .requested_at
        .ok_or_else(|| status(StoreError::Invalid("discovery request time")))?;
    let millis = time
        .seconds
        .checked_mul(1000)
        .and_then(|value| value.checked_add(i64::from(time.nanos) / 1_000_000))
        .ok_or_else(|| status(StoreError::Invalid("discovery request time")))?;
    if context.actor.as_ref() != Some(actor) || millis > now || now - millis >= 30_000 {
        return Err(status(StoreError::AdmissionDenied));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_bounds() {
        assert!(validate_deadline(&Request::new(())).is_err());
        for value in ["0n", "121S", "99999999H", "1", "1X", "-1S", "123456789n"] {
            let mut request = Request::new(());
            request
                .metadata_mut()
                .insert("grpc-timeout", value.parse().unwrap());
            assert!(validate_deadline(&request).is_err(), "{value}");
        }
        for value in ["1n", "1u", "1m", "120S", "2M"] {
            let mut request = Request::new(());
            request
                .metadata_mut()
                .insert("grpc-timeout", value.parse().unwrap());
            assert!(validate_deadline(&request).is_ok(), "{value}");
        }
    }
}
