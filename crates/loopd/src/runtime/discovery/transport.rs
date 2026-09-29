use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use loop_protocol::wire::{provider::v1 as provider, v1};
use prost::Message;
use serde_json::json;
use sha2::{Digest, Sha256};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

use super::ProviderConfig;
use crate::runtime::deployment::read_file;
use crate::store::{ModelStep, StoreError, StoreResult, model_money};

pub(super) struct ProviderConnection {
    pub(super) actor: v1::Actor,
    pub(super) sha256: String,
    endpoint: Endpoint,
}

impl ProviderConnection {
    pub(super) fn open(config: ProviderConfig) -> StoreResult<Self> {
        let endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|_| StoreError::Invalid("provider endpoint"))?;
        let uri = endpoint.uri();
        if uri.scheme_str() != Some("https")
            || uri
                .authority()
                .is_none_or(|authority| authority.as_str().contains('@'))
            || uri.query().is_some()
            || uri.path() != "/"
            || config.domain.is_empty()
            || config.domain.len() > 253
        {
            return Err(StoreError::Invalid("provider HTTPS endpoint"));
        }
        let ca = read_file(&config.ca_file, false, 131_072)?;
        let certificate = read_file(&config.certificate_file, false, 131_072)?;
        let key = read_file(&config.private_key_file, true, 131_072)?;
        let actor = v1::Actor {
            actor_id: Some(v1::ActorId {
                value: config.actor_id,
            }),
            kind: v1::ActorKind::Agent as i32,
            display_name: config.display_name,
            authenticated_subject: config.subject,
        };
        let identity = serde_json::to_vec(&json!({
            "schema": "loop.provider-connector/v1",
            "endpoint": config.endpoint,
            "domain": config.domain,
            "actor": STANDARD.encode(actor.encode_to_vec()),
            "certificate_sha256": format!("{:x}", Sha256::digest(&certificate)),
            "ca_sha256": format!("{:x}", Sha256::digest(&ca)),
        }))
        .map_err(|_| StoreError::Invalid("provider identity"))?;
        let endpoint = endpoint
            .connect_timeout(Duration::from_secs(5))
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name(config.domain)
                    .ca_certificate(Certificate::from_pem(ca))
                    .identity(Identity::from_pem(certificate, key)),
            )
            .map_err(|_| StoreError::Invalid("provider TLS"))?;
        Ok(Self {
            actor,
            sha256: format!("sha256:{:x}", Sha256::digest(identity)),
            endpoint,
        })
    }

    pub(super) async fn invoke(
        &self,
        request: provider::InvokeModelRequest,
        timeout: Duration,
    ) -> StoreResult<v1::ModelResponse> {
        let operation = async {
            let channel = self
                .endpoint
                .clone()
                .connect()
                .await
                .map_err(|_| unavailable())?;
            let mut client = provider::provider_service_client::ProviderServiceClient::new(channel)
                .max_decoding_message_size(1_048_576)
                .max_encoding_message_size(1_048_576);
            let mut request = tonic::Request::new(request);
            request.set_timeout(timeout);
            client
                .invoke_model(request)
                .await
                .map_err(|_| unavailable())?
                .into_inner()
                .response
                .ok_or(StoreError::Corrupt("missing provider response"))
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| unavailable())?
    }

    pub(super) async fn lookup(
        &self,
        step: &ModelStep,
        context: v1::CommandContext,
        timeout: Duration,
    ) -> StoreResult<Option<v1::ModelResponse>> {
        let original = step
            .request
            .context
            .as_ref()
            .ok_or(StoreError::Corrupt("model context"))?;
        let request = provider::LookupInvocationRequest {
            context: Some(context),
            original_request_id: original.request_id.clone(),
            original_idempotency_key: original.idempotency_key.clone(),
            request_sha256: Some(v1::Sha256Digest {
                value: step.request_sha256.to_vec(),
            }),
        };
        let operation = async {
            let channel = self
                .endpoint
                .clone()
                .connect()
                .await
                .map_err(|_| unavailable())?;
            let mut client = provider::provider_service_client::ProviderServiceClient::new(channel)
                .max_decoding_message_size(1_048_576)
                .max_encoding_message_size(1_048_576);
            let mut request = tonic::Request::new(request);
            request.set_timeout(timeout);
            let result = client
                .lookup_invocation(request)
                .await
                .map_err(|_| unavailable())?
                .into_inner();
            let cost = result
                .reserved_cost
                .as_ref()
                .map(|cost| model_money(Some(cost)))
                .transpose()?;
            if cost.is_some_and(|cost| cost > step.reserved_nano_usd) {
                return Err(StoreError::Corrupt("provider reservation mismatch"));
            }
            match provider::InvocationState::try_from(result.state) {
                Ok(provider::InvocationState::Completed) if cost.is_some() => result
                    .response
                    .map(Some)
                    .ok_or(StoreError::Corrupt("provider completed receipt")),
                Ok(provider::InvocationState::Absent)
                    if result.response.is_none() && cost.is_none() =>
                {
                    Ok(None)
                }
                Ok(provider::InvocationState::Ambiguous) if result.response.is_none() => Ok(None),
                _ => Err(StoreError::Corrupt("provider receipt state")),
            }
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| unavailable())?
    }
}

fn unavailable() -> StoreError {
    StoreError::Unavailable("provider invocation evidence")
}
