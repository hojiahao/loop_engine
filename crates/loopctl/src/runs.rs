use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use loop_protocol::wire::{runs::v1 as wire, v1};
use prost::Message;
use uuid::Uuid;

use crate::config::{Connection, Profile, read_file, valid_id};
use crate::discovery::{context_with, request};
use crate::output::{self, Failure};

pub(crate) const SCHEMA: &str = "loop.run-cli/v1";

#[derive(Args)]
pub(crate) struct Arguments {
    /// Absolute path to a private loop.operator/v1 JSON configuration.
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
    /// Start the existing immutable run plan referenced by a canonical protobuf.
    Start {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        key: String,
    },
    /// Execute the current child and advance at most one durable round.
    Step {
        #[arg(long)]
        run: String,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        revision: u64,
        /// Reuse this key and exact original inputs after an uncertain response.
        #[arg(long)]
        key: String,
    },
    /// Read this owner's run and its conservative reservations.
    Status {
        #[arg(long)]
        run: String,
    },
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Start { .. } => "start",
            Self::Step { .. } => "step",
            Self::Status { .. } => "status",
        }
    }

    fn run_id(&self) -> Option<&str> {
        match self {
            Self::Start { .. } => None,
            Self::Step { run, .. } | Self::Status { run } => Some(run),
        }
    }

    fn key(&self) -> String {
        match self {
            Self::Start { key, .. } | Self::Step { key, .. } => key.clone(),
            Self::Status { .. } => Uuid::new_v4().to_string(),
        }
    }
}

pub(crate) async fn run(arguments: Arguments) -> Result<u8, Failure> {
    let timeout = Duration::from_secs(arguments.timeout_seconds);
    let key = arguments.command.key();
    if !valid_id(&key) || arguments.command.run_id().is_some_and(|id| !valid_id(id)) {
        return Err(Failure::Arguments);
    }
    let connection = Connection::load(&arguments.config, timeout, Profile::Operator)?;
    let context = context_with(connection.actor.clone(), key, b"loop.run-cli.command/v1\0")?;
    let plan = match &arguments.command {
        Command::Start { plan, .. } => Some(read_plan(plan)?),
        _ => None,
    };
    let (document, code) = tokio::time::timeout(
        timeout,
        invoke(&arguments.command, connection, context, plan, timeout),
    )
    .await
    .map_err(|_| Failure::Timeout)??;
    if let Some(expected) = arguments.command.run_id()
        && document["run"]["run_id"].as_str() != Some(expected)
    {
        return Err(Failure::Protocol);
    }
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &document).map_err(|_| Failure::Internal)?;
    writeln!(stdout).map_err(|_| Failure::Internal)?;
    Ok(code)
}

fn read_plan(path: &Path) -> Result<v1::PolicyReference, Failure> {
    let bytes = read_file(path, true, 4_096).map_err(|_| Failure::Input)?;
    decode_plan(&bytes)
}

fn decode_plan(bytes: &[u8]) -> Result<v1::PolicyReference, Failure> {
    let plan = v1::PolicyReference::decode(bytes).map_err(|_| Failure::Input)?;
    if plan.encode_to_vec() != bytes {
        return Err(Failure::Input);
    }
    let id = &plan.policy_id.as_ref().ok_or(Failure::Input)?.value;
    if !id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || id.len() > 128
        || !id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
        })
        || plan.revision.starts_with('0')
        || !plan.revision.bytes().all(|byte| byte.is_ascii_digit())
        || plan.revision.parse::<u64>().is_err()
        || plan
            .sha256
            .as_ref()
            .is_none_or(|digest| digest.value.len() != 32)
    {
        return Err(Failure::Input);
    }
    Ok(plan)
}

async fn invoke(
    command: &Command,
    connection: Connection,
    context: v1::CommandContext,
    plan: Option<v1::PolicyReference>,
    timeout: Duration,
) -> Result<(serde_json::Value, u8), Failure> {
    let channel = connection
        .endpoint
        .connect()
        .await
        .map_err(|_| Failure::Transport)?;
    let mut client = wire::run_service_client::RunServiceClient::new(channel)
        .max_decoding_message_size(1_048_576)
        .max_encoding_message_size(1_048_576);
    let context = Some(context);
    let run_id = command.run_id().map(|value| v1::RunId {
        value: value.to_owned(),
    });
    let view = match command {
        Command::Start { .. } => {
            client
                .start_run(request(wire::StartRunRequest { context, plan }, timeout))
                .await
                .map_err(Failure::from_status)?
                .into_inner()
                .run
        }
        Command::Step { revision, .. } => {
            client
                .step_run(request(
                    wire::StepRunRequest {
                        context,
                        run_id,
                        expected_revision: *revision,
                    },
                    timeout,
                ))
                .await
                .map_err(Failure::from_status)?
                .into_inner()
                .run
        }
        Command::Status { .. } => {
            client
                .get_run(request(wire::GetRunRequest { context, run_id }, timeout))
                .await
                .map_err(Failure::from_status)?
                .into_inner()
                .run
        }
    };
    render(command.name(), view)
}

fn render(command: &str, view: Option<wire::RunView>) -> Result<(serde_json::Value, u8), Failure> {
    let view = view.ok_or(Failure::Protocol)?;
    loop_protocol::runs::validate_view(&view).map_err(|_| Failure::Protocol)?;
    let status = wire::RunStatus::try_from(view.status).map_err(|_| Failure::Protocol)?;
    let (status, code) = match status {
        wire::RunStatus::Active => ("active", 0),
        wire::RunStatus::Completed => ("completed", 0),
        wire::RunStatus::BudgetExhausted => ("budget_exhausted", 7),
        wire::RunStatus::InfrastructureFailed => ("infrastructure_failed", 7),
        wire::RunStatus::DeadlineExceeded => ("deadline_exceeded", 7),
        wire::RunStatus::Unspecified => return Err(Failure::Protocol),
    };
    let (child, _) = output::handle(view.current_job.ok_or(Failure::Protocol)?)?;
    let budget = view.budget.ok_or(Failure::Protocol)?;
    let wall = budget.maximum_wall_time.ok_or(Failure::Protocol)?;
    Ok((
        serde_json::json!({"schema":SCHEMA,"command":command,"run":{
            "run_id":view.run_id.ok_or(Failure::Protocol)?.value,
            "status":status,"revision":view.revision.to_string(),
            "maximum_rounds":view.maximum_rounds.to_string(),
            "completed_rounds":view.completed_rounds.to_string(),
            "current_job":child,
            "budget":{
                "maximum_steps":budget.maximum_steps.to_string(),
                "maximum_input_tokens":budget.maximum_input_tokens.to_string(),
                "maximum_output_tokens":budget.maximum_output_tokens.to_string(),
                "maximum_cost":output::money(budget.maximum_cost.ok_or(Failure::Protocol)?)?,
                "maximum_wall_time":{"seconds":wall.seconds.to_string(),"nanos":wall.nanos.to_string()}
            },
            "reserved_steps":view.reserved_steps.to_string(),
            "reserved_input_tokens":view.reserved_input_tokens.to_string(),
            "reserved_output_tokens":view.reserved_output_tokens.to_string(),
            "reserved_cost":output::money(view.reserved_cost.ok_or(Failure::Protocol)?)?,
            "submitted_at":output::timestamp(view.submitted_at.ok_or(Failure::Protocol)?)?,
            "updated_at":output::timestamp(view.updated_at.ok_or(Failure::Protocol)?)?,
            "deadline":output::timestamp(view.deadline.ok_or(Failure::Protocol)?)?,
            "plan_verified":view.plan_verified
        }}),
        code,
    ))
}

#[cfg(test)]
mod tests;
