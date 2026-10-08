use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Args, Subcommand};
use loop_protocol::wire::{discovery::v1 as wire, v1};
use prost::Message;
use sha2::{Digest, Sha256};
use tonic::Request;
use uuid::Uuid;

use crate::config::{Connection, read_file, valid_id};
use crate::output::{self, Failure};

#[derive(Args)]
pub(crate) struct Arguments {
    /// Absolute path to a private loop.client/v1 JSON configuration.
    #[arg(long)]
    config: PathBuf,
    /// Total connection and RPC deadline; commands are never retried automatically.
    #[arg(long, global = true, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=120))]
    timeout_seconds: u64,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Submit a bounded canonical DiscoveryJobInput protobuf.
    Start {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        key: String,
    },
    /// Execute the frozen plan under the supplied revision.
    Execute(Mutation),
    /// Read the job and its conservative reservations.
    Status {
        #[arg(long)]
        job: String,
    },
    /// Pause durable execution without resetting its budget.
    Pause(Mutation),
    /// Request durable cancellation; Ctrl-C only interrupts this client.
    Cancel(Mutation),
    /// Resume the original job and its existing reservations.
    Resume(Mutation),
    /// Confirm expiry after the original absolute deadline.
    Expire(Mutation),
    /// Recover evidence without authorizing another paid call.
    Reconcile(Mutation),
    /// Read an allowlisted page of job events, not a global ledger proof.
    Events {
        #[arg(long)]
        job: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
}

#[derive(Args)]
struct Mutation {
    #[arg(long)]
    job: String,
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    revision: u64,
    /// Reuse this key and the exact original inputs after an uncertain response.
    #[arg(long)]
    key: String,
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Start { .. } => "start",
            Self::Execute(_) => "execute",
            Self::Status { .. } => "status",
            Self::Pause(_) => "pause",
            Self::Cancel(_) => "cancel",
            Self::Resume(_) => "resume",
            Self::Expire(_) => "expire",
            Self::Reconcile(_) => "reconcile",
            Self::Events { .. } => "events",
        }
    }

    fn job(&self) -> Option<&str> {
        match self {
            Self::Start { .. } => None,
            Self::Status { job } | Self::Events { job, .. } => Some(job),
            Self::Execute(value)
            | Self::Pause(value)
            | Self::Cancel(value)
            | Self::Resume(value)
            | Self::Expire(value)
            | Self::Reconcile(value) => Some(&value.job),
        }
    }

    fn key(&self) -> String {
        match self {
            Self::Start { key, .. } => key.clone(),
            Self::Status { .. } | Self::Events { .. } => Uuid::new_v4().to_string(),
            Self::Execute(value)
            | Self::Pause(value)
            | Self::Cancel(value)
            | Self::Resume(value)
            | Self::Expire(value)
            | Self::Reconcile(value) => value.key.clone(),
        }
    }
}

pub(crate) async fn run(arguments: Arguments) -> Result<u8, Failure> {
    let timeout = Duration::from_secs(arguments.timeout_seconds);
    let key = arguments.command.key();
    if !valid_id(&key) || arguments.command.job().is_some_and(|job| !valid_id(job)) {
        return Err(Failure::Arguments);
    }
    let connection = Connection::load(&arguments.config, timeout)?;
    let context = context(connection.actor.clone(), key)?;
    let input = match &arguments.command {
        Command::Start { input, .. } => Some(read_input(
            input,
            context.requested_at.as_ref().ok_or(Failure::Internal)?,
        )?),
        _ => None,
    };
    let (document, code) = tokio::time::timeout(
        timeout,
        invoke(&arguments.command, connection, context, input, timeout),
    )
    .await
    .map_err(|_| Failure::Timeout)??;
    if let Some(expected) = arguments.command.job() {
        let actual = document
            .get("job")
            .and_then(|job| job.get("job_id"))
            .or_else(|| document.get("job_id"))
            .and_then(serde_json::Value::as_str);
        if actual != Some(expected) {
            return Err(Failure::Protocol);
        }
    }
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &document).map_err(|_| Failure::Internal)?;
    writeln!(stdout).map_err(|_| Failure::Internal)?;
    Ok(code)
}

fn context(actor: v1::Actor, key: String) -> Result<v1::CommandContext, Failure> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Failure::Internal)?;
    let seconds = i64::try_from(now.as_secs()).map_err(|_| Failure::Internal)?;
    let mut digest = Sha256::new();
    digest.update(b"loop.discovery-cli.command/v1\0");
    digest.update(key.as_bytes());
    let digest = format!("{:x}", digest.finalize());
    Ok(v1::CommandContext {
        request_id: Some(v1::RequestId {
            value: Uuid::new_v4().to_string(),
        }),
        correlation_id: Some(v1::CorrelationId {
            value: format!("cli.correlation.{digest}"),
        }),
        causation_id: Some(v1::CausationId {
            value: format!("cli.causation.{digest}"),
        }),
        idempotency_key: Some(v1::IdempotencyKey { value: key }),
        actor: Some(actor),
        requested_at: Some(prost_types::Timestamp {
            seconds,
            nanos: now.subsec_nanos() as i32,
        }),
    })
}

fn read_input(
    path: &Path,
    submitted_at: &prost_types::Timestamp,
) -> Result<wire::DiscoveryJobInput, Failure> {
    let bytes = read_file(path, false, 1_048_576).map_err(|_| Failure::Input)?;
    decode_input(&bytes, submitted_at)
}

fn decode_input(
    bytes: &[u8],
    submitted_at: &prost_types::Timestamp,
) -> Result<wire::DiscoveryJobInput, Failure> {
    let input = wire::DiscoveryJobInput::decode(bytes).map_err(|_| Failure::Input)?;
    // The exact round trip rejects unknown fields, duplicate fields, alternate
    // order and nonminimal wire encodings before any network request is sent.
    if input.encode_to_vec() != bytes {
        return Err(Failure::Input);
    }
    loop_protocol::job::validate_discovery_input(&input, submitted_at)
        .map_err(|_| Failure::Input)?;
    Ok(input)
}

fn request<T>(message: T, timeout: Duration) -> Request<T> {
    let mut request = Request::new(message);
    request.set_timeout(timeout);
    request
}

async fn invoke(
    command: &Command,
    connection: Connection,
    context: v1::CommandContext,
    input: Option<wire::DiscoveryJobInput>,
    timeout: Duration,
) -> Result<(serde_json::Value, u8), Failure> {
    let channel = connection
        .endpoint
        .connect()
        .await
        .map_err(|_| Failure::Transport)?;
    let mut client = wire::discovery_service_client::DiscoveryServiceClient::new(channel)
        .max_decoding_message_size(1_048_576)
        .max_encoding_message_size(1_048_576);
    let context = Some(context);
    let job_id = command.job().map(|value| v1::JobId {
        value: value.to_owned(),
    });
    match command {
        Command::Start { .. } => {
            let response = client
                .start_discovery(request(
                    wire::StartDiscoveryRequest {
                        context,
                        discovery: input,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::job(command.name(), response.job)
        }
        Command::Execute(value) => {
            let response = client
                .execute_discovery(request(
                    wire::ExecuteDiscoveryRequest {
                        context,
                        job_id,
                        expected_revision: value.revision,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::step(command.name(), response.step)
        }
        Command::Status { .. } => {
            let response = client
                .get_discovery(request(
                    wire::GetDiscoveryRequest { context, job_id },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::step(command.name(), response.step)
        }
        Command::Pause(value) => {
            let response = client
                .pause_discovery(request(
                    wire::PauseDiscoveryRequest {
                        context,
                        job_id,
                        expected_revision: value.revision,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::job(command.name(), response.job)
        }
        Command::Cancel(value) => {
            let response = client
                .cancel_discovery(request(
                    wire::CancelDiscoveryRequest {
                        context,
                        job_id,
                        expected_revision: value.revision,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::job(command.name(), response.job)
        }
        Command::Expire(value) => {
            let response = client
                .expire_discovery(request(
                    wire::ExpireDiscoveryRequest {
                        context,
                        job_id,
                        expected_revision: value.revision,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::job(command.name(), response.job)
        }
        Command::Resume(value) => {
            let response = client
                .resume_discovery(request(
                    wire::ResumeDiscoveryRequest {
                        context,
                        job_id,
                        expected_revision: value.revision,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::step(command.name(), response.step)
        }
        Command::Reconcile(value) => {
            let response = client
                .reconcile_discovery(request(
                    wire::ReconcileDiscoveryRequest {
                        context,
                        job_id,
                        expected_revision: value.revision,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::step(command.name(), response.step)
        }
        Command::Events { job, after, limit } => {
            let response = client
                .list_discovery_events(request(
                    wire::ListDiscoveryEventsRequest {
                        context,
                        job_id,
                        after_sequence: *after,
                        limit: *limit,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner();
            output::events(job, *after, *limit, response)
        }
    }
}

#[cfg(test)]
mod tests;
