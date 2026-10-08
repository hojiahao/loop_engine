use loop_core::factor::{CanonicalDecimal, Identifier, PositiveInteger};
use loop_protocol::wire::{discovery::v1 as wire, v1};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::valid_id;

const SCHEMA: &str = "loop.discovery-cli/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    Arguments,
    Configuration,
    Input,
    Authentication,
    Authorization,
    Conflict,
    Timeout,
    Transport,
    RateLimit,
    RemoteCancelled,
    NotFound,
    Protocol,
    Internal,
    Interrupted,
}

impl Failure {
    pub(crate) fn exit_code(self) -> u8 {
        match self {
            Self::Arguments | Self::Configuration | Self::Input => 2,
            Self::Authentication | Self::Authorization => 3,
            Self::Conflict => 4,
            Self::Timeout => 5,
            Self::Transport | Self::RateLimit | Self::RemoteCancelled => 6,
            Self::NotFound | Self::Protocol | Self::Internal => 8,
            Self::Interrupted => 130,
        }
    }

    pub(crate) fn envelope(self) -> Value {
        let category = match self {
            Self::Arguments => "arguments",
            Self::Configuration => "configuration",
            Self::Input => "input",
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Conflict => "conflict",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::RateLimit => "rate_limit",
            Self::RemoteCancelled => "remote_cancelled",
            Self::NotFound => "not_found",
            Self::Protocol => "protocol",
            Self::Internal => "internal",
            Self::Interrupted => "interrupted",
        };
        json!({"schema":SCHEMA,"error":{"category":category}})
    }

    pub(crate) fn from_status(status: tonic::Status) -> Self {
        // Never serialize status.message(), details, metadata or source chains.
        match status.code() {
            tonic::Code::InvalidArgument | tonic::Code::OutOfRange => Self::Input,
            tonic::Code::Unauthenticated => Self::Authentication,
            tonic::Code::PermissionDenied => Self::Authorization,
            tonic::Code::AlreadyExists | tonic::Code::Aborted | tonic::Code::FailedPrecondition => {
                Self::Conflict
            }
            tonic::Code::DeadlineExceeded => Self::Timeout,
            tonic::Code::Unavailable => Self::Transport,
            tonic::Code::ResourceExhausted => Self::RateLimit,
            tonic::Code::Cancelled => Self::RemoteCancelled,
            tonic::Code::NotFound => Self::NotFound,
            tonic::Code::Internal | tonic::Code::Unknown => Self::Internal,
            _ => Self::Protocol,
        }
    }
}

pub(crate) fn job(
    command: &str,
    job: Option<wire::DiscoveryJobHandle>,
) -> Result<(Value, u8), Failure> {
    let (job, code) = handle(job.ok_or(Failure::Protocol)?)?;
    Ok((json!({"schema":SCHEMA,"command":command,"job":job}), code))
}

fn handle(job: wire::DiscoveryJobHandle) -> Result<(Value, u8), Failure> {
    let id = job.job_id.ok_or(Failure::Protocol)?.value;
    if !valid_id(&id) || job.revision == 0 {
        return Err(Failure::Protocol);
    }
    let submitted_at = job.submitted_at.ok_or(Failure::Protocol)?;
    let updated_at = job.updated_at.ok_or(Failure::Protocol)?;
    if (updated_at.seconds, updated_at.nanos) < (submitted_at.seconds, submitted_at.nanos) {
        return Err(Failure::Protocol);
    }
    use wire::DiscoveryJobStatus as Status;
    let (status, code) = match Status::try_from(job.status).map_err(|_| Failure::Protocol)? {
        Status::Queued => ("queued", 0),
        Status::Leased => ("leased", 0),
        Status::Running => ("running", 0),
        Status::Succeeded => ("succeeded", 0),
        Status::FactorRejected => ("factor_rejected", 7),
        Status::InfrastructureFailed => ("infrastructure_failed", 7),
        Status::Cancelled => ("cancelled", 7),
        Status::BudgetExhausted => ("budget_exhausted", 7),
        Status::Paused => ("paused", 0),
        Status::Unspecified => return Err(Failure::Protocol),
    };
    Ok((
        json!({"job_id":id,"status":status,"revision":job.revision.to_string(),
        "submitted_at":timestamp(submitted_at)?,"updated_at":timestamp(updated_at)?}),
        code,
    ))
}

pub(crate) fn step(
    command: &str,
    step: Option<wire::DiscoveryStepView>,
) -> Result<(Value, u8), Failure> {
    let step = step.ok_or(Failure::Protocol)?;
    let source = step.job.ok_or(Failure::Protocol)?;
    let succeeded = source.status == wire::DiscoveryJobStatus::Succeeded as i32;
    let (job, code) = handle(source)?;
    use wire::DiscoveryStepState as State;
    let state = match State::try_from(step.state).map_err(|_| Failure::Protocol)? {
        State::Unspecified => "unspecified",
        State::Reserved => "reserved",
        State::Dispatched => "dispatched",
        State::Completed => "completed",
        State::Ambiguous => "ambiguous",
    };
    let reserved_cost = step.reserved_cost.map(money).transpose()?;
    if reserved_cost.is_none()
        && (step.reserved_input_tokens != 0 || step.reserved_output_tokens != 0)
    {
        return Err(Failure::Protocol);
    }
    let mut view = json!({"state":state,"plan_verified":step.plan_verified,
        "reserved_cost":reserved_cost,
        "reserved_input_tokens":step.reserved_input_tokens.to_string(),
        "reserved_output_tokens":step.reserved_output_tokens.to_string()});
    if let Some(value) = step.candidate {
        if !step.plan_verified
            || !succeeded
            || step.state != State::Completed as i32
            || command == "reconcile"
        {
            return Err(Failure::Protocol);
        }
        view["candidate"] = candidate(value)?;
    }
    Ok((
        json!({"schema":SCHEMA,"command":command,"job":job,"step":view}),
        code,
    ))
}

fn money(value: v1::Money) -> Result<Value, Failure> {
    let amount = value.amount.ok_or(Failure::Protocol)?.value;
    if amount.len() > 128
        || amount.starts_with('-')
        || value.currency_code != "USD"
        || CanonicalDecimal::new(amount.clone()).is_err()
    {
        return Err(Failure::Protocol);
    }
    Ok(json!({"amount":amount,"currency_code":value.currency_code}))
}

fn timestamp(value: prost_types::Timestamp) -> Result<Value, Failure> {
    if !(-62_135_596_800..=253_402_300_799).contains(&value.seconds)
        || !(0..1_000_000_000).contains(&value.nanos)
    {
        return Err(Failure::Protocol);
    }
    Ok(json!({"seconds":value.seconds.to_string(),"nanos":value.nanos.to_string()}))
}

pub(crate) fn events(
    job: &str,
    after: u64,
    limit: u32,
    response: wire::ListDiscoveryEventsResponse,
) -> Result<(Value, u8), Failure> {
    if response.events.len() > limit as usize
        || (response.has_more && response.events.len() != limit as usize)
    {
        return Err(Failure::Protocol);
    }
    let mut cursor = after;
    let mut events = Vec::with_capacity(response.events.len());
    for event in response.events {
        if event.sequence <= cursor {
            return Err(Failure::Protocol);
        }
        cursor = event.sequence;
        events.push(json!({"sequence":event.sequence.to_string(),
            "occurred_at":timestamp(event.occurred_at.ok_or(Failure::Protocol)?)?,
            "operation":operation(event.operation)?}));
    }
    if response.next_after_sequence != cursor {
        return Err(Failure::Protocol);
    }
    Ok((
        json!({"schema":SCHEMA,"command":"events","job_id":job,"events":events,
        "next_after_sequence":cursor.to_string(),"has_more":response.has_more}),
        0,
    ))
}

fn operation(value: i32) -> Result<&'static str, Failure> {
    use wire::DiscoveryOperation as Operation;
    Ok(
        match Operation::try_from(value).map_err(|_| Failure::Protocol)? {
            Operation::Start => "start",
            Operation::Reserve => "reserve",
            Operation::Dispatch => "dispatch",
            Operation::Uncertain => "uncertain",
            Operation::Finish => "finish",
            Operation::Call => "call",
            Operation::ToolRecord => "tool_record",
            Operation::Takeover => "takeover",
            Operation::Resume => "resume",
            Operation::Retry => "retry",
            Operation::Fail => "fail",
            Operation::Reconcile => "reconcile",
            Operation::Pause => "pause",
            Operation::Cancel => "cancel",
            Operation::Expire => "expire",
            Operation::Unspecified => return Err(Failure::Protocol),
        },
    )
}

fn candidate(value: wire::DiscoveryCandidate) -> Result<Value, Failure> {
    if value.canonicalization_profile != "loop.factor-ast/v1"
        || value.canonical_json.is_empty()
        || value.canonical_json.len() > 262_144
    {
        return Err(Failure::Protocol);
    }
    let node: Node =
        serde_json::from_slice(&value.canonical_json).map_err(|_| Failure::Protocol)?;
    let mut count = 0;
    node.validate(1, &mut count)?;
    if serde_json::to_vec(&node).map_err(|_| Failure::Protocol)? != value.canonical_json {
        return Err(Failure::Protocol);
    }
    let id = value.expression_id.ok_or(Failure::Protocol)?.value;
    let mut hash = Sha256::new();
    hash.update(b"loop.factor-ast/v1\0");
    hash.update(&value.canonical_json);
    if id != format!("sha256:{:x}", hash.finalize()) {
        return Err(Failure::Protocol);
    }
    let canonical = String::from_utf8(value.canonical_json).map_err(|_| Failure::Protocol)?;
    Ok(
        json!({"expression_id":id,"canonicalization_profile":value.canonicalization_profile,
        "canonical_json":canonical}),
    )
}

// Only canonical AST fields can cross the CLI output boundary. This validates
// shape and identity; the service owns the frozen registry and semantic proof.
#[derive(Deserialize, Serialize)]
#[serde(tag = "node", deny_unknown_fields)]
enum Node {
    #[serde(rename = "field")]
    Field { field: Identifier },
    #[serde(rename = "decimal")]
    Decimal { value: CanonicalDecimal },
    #[serde(rename = "boolean")]
    Boolean { value: bool },
    #[serde(rename = "enum")]
    Enumeration {
        enum_type: Identifier,
        value: Identifier,
    },
    #[serde(rename = "call")]
    Call {
        operator: Identifier,
        operator_version: PositiveInteger,
        arguments: Vec<Node>,
    },
}

impl Node {
    fn validate(&self, depth: usize, count: &mut usize) -> Result<(), Failure> {
        *count += 1;
        if depth > 64 || *count > 4_096 {
            return Err(Failure::Protocol);
        }
        if let Self::Call { arguments, .. } = self {
            if arguments.len() > 1_024 {
                return Err(Failure::Protocol);
            }
            for argument in arguments {
                argument.validate(depth + 1, count)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
