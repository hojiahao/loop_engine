use loop_protocol::wire::discovery::v1::{
    self as wire, discovery_service_server::DiscoveryService,
};
use tonic::{Request, Response, Status};

use super::{RuntimeService, status};
use crate::runtime::Role;
use crate::store::{JobRepository, RoleCommand, RoleJobHandle, StoreError};

#[tonic::async_trait]
impl DiscoveryService for RuntimeService {
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
        let id = &command
            .job_id
            .as_ref()
            .ok_or_else(|| status(StoreError::Invalid("discovery job ID")))?
            .value;
        let step = self
            .store
            .model_step(&principal.actor, id)
            .await
            .map_err(status)?;
        // The second store read is the authoritative same-transaction projection;
        // a concurrent completion must not combine an old job with a newer step.
        let current = step.as_ref().map_or(&job, |step| &step.job);
        let view = executor.view(current, step.as_ref()).map_err(status)?;
        Ok(Response::new(wire::GetDiscoveryResponse {
            step: Some(view),
        }))
    }
}

fn validate_deadline<T>(request: &Request<T>) -> Result<(), Status> {
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

fn validate_context(
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
