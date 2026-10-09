use std::path::{Path, PathBuf};
use std::sync::Arc;

use loop_core::factor::{
    ExpressionId, OperatorPolicyRegistry, PolicyId, PositiveInteger, us_equities,
};
use loop_protocol::job::{protocol_selection_sha256, validate_job_specification};
use loop_protocol::wire::{discovery::v1::DiscoveryJobInput, v1 as wire};
use prost::Message;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::manifests::ObjectRef;
use crate::runtime::{deployment::read_file, model_codec};
use crate::store::{StoreError, StoreResult, model_duration, model_money, validate_id};

const MAX_OBJECT: u64 = 1_048_576;
// The published Phase 10 context descriptor (8199a78) remains compatible with
// these closed v1/v2 plans. New lifecycle RPCs do not rewrite frozen plan pins.
const CONTEXT_DESCRIPTOR: [u8; 32] = [
    0x2c, 0x60, 0xbd, 0x74, 0x3b, 0x8f, 0x4d, 0xb0, 0x9f, 0xdb, 0x19, 0x20, 0x0e, 0xf8, 0x30, 0x7d,
    0x49, 0xad, 0xc3, 0xb2, 0x7a, 0xdf, 0x28, 0xbf, 0xf6, 0x81, 0x84, 0x58, 0x98, 0xbd, 0x3e, 0x1c,
];
// The published lifecycle descriptor (12768ff) remains explicitly supported.
// Observation fields do not alter the frozen execution inputs or plan meaning.
const LIFECYCLE_DESCRIPTOR: [u8; 32] = [
    0x18, 0xf2, 0xfc, 0x91, 0x66, 0x47, 0x20, 0x84, 0x4b, 0x87, 0xce, 0xf5, 0x99, 0x00, 0x46, 0x23,
    0x7b, 0xd6, 0x2d, 0x8c, 0x3f, 0x81, 0x43, 0x0a, 0x6c, 0x5f, 0x6e, 0xa7, 0x99, 0xbf, 0xa9, 0xd5,
];
// The published operational descriptor (b692e30) keeps identical Discovery
// execution semantics when the separate RunService is added.
const OPERATIONS_DESCRIPTOR: [u8; 32] = [
    0x15, 0x60, 0xea, 0x74, 0x46, 0x17, 0xc0, 0x7d, 0x3f, 0x74, 0x90, 0x73, 0x17, 0x0d, 0xb0, 0xf7,
    0x7c, 0x50, 0x00, 0x0e, 0xa6, 0xe2, 0xaa, 0x38, 0xcb, 0x20, 0xbf, 0xb3, 0xe2, 0x9d, 0xb4, 0x8f,
];
const FEATURES: [&str; 5] = [
    "discovery.model-step.v1",
    "jobs.envelope.v1",
    "jobs.kind-input.v1",
    "jobs.prelease-terminal.v1",
    "provider.invocation-lookup.v1",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanDocument {
    schema: String,
    id: String,
    revision: String,
    actor_id: String,
    run_id: String,
    provider_sha256: String,
    input: ObjectRef,
    invocation: ObjectRef,
    #[serde(default)]
    tool_invocation: Option<ObjectRef>,
    registry: ObjectRef,
    data: ObjectRef,
    protocol: ObjectRef,
}

/// An administrator-pinned, bounded discovery plan; not caller-supplied authority.
/// Content is rechecked before each use and never repaired or overwritten.
pub(super) struct FrozenPlan {
    pub(super) actor_id: String,
    pub(super) run_id: String,
    pub(super) provider_sha256: String,
    pub(super) input: DiscoveryJobInput,
    pub(super) invocation: wire::ModelInvocation,
    pub(super) tool_invocation: Option<wire::ModelInvocation>,
    pub(super) registry: Arc<OperatorPolicyRegistry>,
    pub(super) data: ObjectRef,
    pub(super) protocol: wire::ProtocolSelectionSnapshot,
    files: Vec<(PathBuf, ObjectRef)>,
}

impl FrozenPlan {
    /// Load bounded private files from a flat, trusted content-addressed root.
    /// Unknown fields, unknown protobuf extensions, changed bytes and unsupported
    /// execution profiles fail closed. Dataset access is separately authorized by
    /// the existing development resolver; this loader never opens dataset files.
    pub(super) fn load(root: &Path, reference: &ObjectRef) -> StoreResult<Self> {
        let mut files = Vec::new();
        let bytes = load_object(root, reference, &mut files)?;
        let document: PlanDocument = serde_json::from_slice(&bytes)
            .map_err(|_| StoreError::Invalid("discovery plan JSON"))?;
        let controlled = document.schema == "loop.discovery-plan/v2";
        if (!controlled && document.schema != "loop.discovery-plan/v1")
            || controlled != document.tool_invocation.is_some()
        {
            return Err(StoreError::Invalid("discovery plan version"));
        }
        validate_id(&document.actor_id)?;
        validate_id(&document.run_id)?;
        ExpressionId::parse(&document.provider_sha256)
            .map_err(|_| StoreError::Invalid("discovery provider identity"))?;
        let data_digest = document.data.digest()?;
        if !(1..=MAX_OBJECT).contains(&document.data.byte_size) {
            return Err(StoreError::Invalid("discovery data manifest size"));
        }
        let mut input: DiscoveryJobInput = decode_object(root, &document.input, &mut files)?;
        let invocation: wire::ModelInvocation =
            decode_object(root, &document.invocation, &mut files)?;
        let tool_invocation: Option<wire::ModelInvocation> = document
            .tool_invocation
            .as_ref()
            .map(|reference| decode_object(root, reference, &mut files))
            .transpose()?;
        let protocol: wire::ProtocolSelectionSnapshot =
            decode_object(root, &document.protocol, &mut files)?;
        let registry_bytes = load_object(root, &document.registry, &mut files)?;
        let registry = Arc::new(
            us_equities::registry()
                .map_err(|_| StoreError::Unavailable("installed operator registry"))?,
        );
        if registry_bytes != registry.canonical_bytes() {
            return Err(StoreError::Invalid("discovery operator registry"));
        }
        if input.research_policy.is_some() {
            return Err(StoreError::Invalid("discovery template policy"));
        }
        let policy = wire::PolicyReference {
            policy_id: Some(wire::PolicyId { value: document.id }),
            revision: document.revision,
            sha256: Some(wire::Sha256Digest {
                value: reference.digest()?.to_vec(),
            }),
        };
        input.research_policy = Some(policy);
        validate_policy(&invocation)?;
        validate_protocol(&protocol, controlled)?;
        if let Some(first) = &tool_invocation {
            validate_policy(first)?;
            model_codec::validate_tools(first, false)?;
            model_codec::validate_tools(&invocation, true)?;
            // Leave room for the two mandatory history messages and the bounded
            // 16 KiB tool document plus its typed envelope/call identity.
            if first.messages.len() > 30
                || first
                    .messages
                    .iter()
                    .map(Message::encoded_len)
                    .sum::<usize>()
                    > 112_640
                || first.request_id.is_some()
                || invocation.request_id.is_some()
                || first.model != invocation.model
                || first.request_policy != invocation.request_policy
                || first.messages != invocation.messages
            {
                return Err(StoreError::Invalid("discovery conversation templates"));
            }
        } else {
            model_codec::validate_template(&invocation)?;
        }
        validate_budgets(&input, &invocation, tool_invocation.as_ref())?;
        if input
            .dataset
            .as_ref()
            .and_then(|data| data.manifest_sha256.as_ref())
            .is_none_or(|digest| digest.value.as_slice() != data_digest)
        {
            return Err(StoreError::Invalid("discovery data binding"));
        }
        let plan = Self {
            actor_id: document.actor_id,
            run_id: document.run_id,
            provider_sha256: document.provider_sha256,
            input,
            invocation,
            tool_invocation,
            registry,
            data: document.data,
            protocol,
            files,
        };
        plan.validate_input()?;
        Ok(plan)
    }

    /// Re-read each pinned small file before dispatch/recovery. The operation is
    /// read-only; absence or corruption never permits a new model request.
    pub(super) fn check(&self) -> StoreResult<()> {
        for (path, reference) in &self.files {
            checked_read(path, reference)?;
        }
        Ok(())
    }

    /// Match the complete immutable plan inputs and owner before an existing job
    /// can dispatch or expose a result. A mismatch never grants retry authority.
    pub(super) fn matches(&self, job: &wire::JobSpecification) -> StoreResult<()> {
        self.check()?;
        if job.kind != wire::JobKind::Discovery as i32
            || job.run_id.as_ref().map(|run| run.value.as_str()) != Some(self.run_id.as_str())
            || job
                .submitted_by
                .as_ref()
                .and_then(|actor| actor.actor_id.as_ref())
                .map(|actor| actor.value.as_str())
                != Some(self.actor_id.as_str())
            || job.protocol_selection.as_ref() != Some(&self.protocol)
            || job.input.as_ref()
                != Some(&wire::job_specification::Input::Discovery(self.job_input()))
        {
            return Err(StoreError::AdmissionDenied);
        }
        validate_job_specification(job)?;
        Ok(())
    }

    fn job_input(&self) -> wire::DiscoveryJobInput {
        wire::DiscoveryJobInput {
            dataset: self.input.dataset.clone(),
            research_policy: self.input.research_policy.clone(),
            maker_model: self.input.maker_model.clone(),
            checker_model: self.input.checker_model.clone(),
            budget: self.input.budget.as_ref().map(|budget| wire::JobBudget {
                maximum_steps: budget.maximum_steps,
                maximum_input_tokens: budget.maximum_input_tokens,
                maximum_output_tokens: budget.maximum_output_tokens,
                maximum_cost: budget.maximum_cost.clone(),
                maximum_wall_time: budget.maximum_wall_time,
            }),
            maximum_candidates: self.input.maximum_candidates,
        }
    }

    fn validate_input(&self) -> StoreResult<()> {
        // Use the existing durable-job validator without manufacturing an actual
        // job or assigning execution authority. Submission validates time again.
        let submitted_at = [
            self.protocol.selected_at,
            self.input
                .maker_model
                .as_ref()
                .and_then(|model| model.resolved_at),
            self.input
                .checker_model
                .as_ref()
                .and_then(|model| model.resolved_at),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|time| (time.seconds, time.nanos));
        let specification = wire::JobSpecification {
            job_id: Some(wire::JobId {
                value: "plan.validation".to_owned(),
            }),
            run_id: Some(wire::RunId {
                value: self.run_id.clone(),
            }),
            kind: wire::JobKind::Discovery as i32,
            input: Some(wire::job_specification::Input::Discovery(self.job_input())),
            submitted_at,
            submitted_by: Some(wire::Actor {
                actor_id: Some(wire::ActorId {
                    value: self.actor_id.clone(),
                }),
                kind: wire::ActorKind::Agent as i32,
                display_name: "Plan validation".to_owned(),
                authenticated_subject: "plan:validation".to_owned(),
            }),
            idempotency_key: Some(wire::IdempotencyKey {
                value: "plan.validation".to_owned(),
            }),
            correlation_id: Some(wire::CorrelationId {
                value: "plan.validation".to_owned(),
            }),
            causation_id: Some(wire::CausationId {
                value: "plan.validation".to_owned(),
            }),
            protocol_selection: Some(self.protocol.clone()),
        };
        validate_job_specification(&specification)?;
        Ok(())
    }
}

fn validate_policy(invocation: &wire::ModelInvocation) -> StoreResult<()> {
    let policy = invocation
        .request_policy
        .as_ref()
        .ok_or(StoreError::Invalid("discovery provider policy"))?;
    PolicyId::new(
        policy
            .policy_id
            .as_ref()
            .ok_or(StoreError::Invalid("discovery provider policy ID"))?
            .value
            .clone(),
    )
    .map_err(|_| StoreError::Invalid("discovery provider policy ID"))?;
    PositiveInteger::new(&policy.revision)
        .map_err(|_| StoreError::Invalid("discovery provider policy revision"))?;
    if policy.revision.parse::<u64>().is_err()
        || policy
            .sha256
            .as_ref()
            .is_none_or(|digest| digest.value.len() != 32)
    {
        return Err(StoreError::Invalid("discovery provider policy binding"));
    }
    Ok(())
}

fn validate_budgets(
    input: &DiscoveryJobInput,
    invocation: &wire::ModelInvocation,
    first: Option<&wire::ModelInvocation>,
) -> StoreResult<()> {
    let job = input
        .budget
        .as_ref()
        .ok_or(StoreError::Invalid("discovery job budget"))?;
    let step = invocation
        .budget
        .as_ref()
        .ok_or(StoreError::Invalid("discovery invocation budget"))?;
    let job_cost = model_money(job.maximum_cost.as_ref())?;
    let step_cost = model_money(step.maximum_cost.as_ref())?;
    let job_wall = model_duration(job.maximum_wall_time.as_ref())?;
    let step_wall = model_duration(step.maximum_wall_time.as_ref())?;
    let job_exact = job
        .maximum_wall_time
        .as_ref()
        .map(|duration| (duration.seconds, duration.nanos));
    let step_exact = step
        .maximum_wall_time
        .as_ref()
        .map(|duration| (duration.seconds, duration.nanos));
    if input.maximum_candidates != 1
        || job.maximum_steps != if first.is_some() { 3 } else { 1 }
        || invocation.model != input.maker_model
        || step.maximum_input_tokens == 0
        || step.maximum_output_tokens == 0
        || step.maximum_input_tokens > job.maximum_input_tokens
        || step.maximum_output_tokens > job.maximum_output_tokens
        || step_cost == 0
        || step_cost > job_cost
        || step_wall
            .checked_add(5_000)
            .is_none_or(|wall| wall >= job_wall)
        || step_exact > job_exact
        || input
            .maker_model
            .as_ref()
            .and_then(|model| model.capabilities.as_ref())
            .is_none_or(|capabilities| !capabilities.supports_structured_output)
    {
        return Err(StoreError::Invalid("discovery frozen budget or model"));
    }
    if let Some(first) = first {
        let first_budget = first
            .budget
            .as_ref()
            .ok_or(StoreError::Invalid("discovery tool budget"))?;
        let first_cost = model_money(first_budget.maximum_cost.as_ref())?;
        let first_wall = model_duration(first_budget.maximum_wall_time.as_ref())?;
        if first_budget.maximum_input_tokens == 0
            || first_budget.maximum_output_tokens == 0
            || first_cost == 0
            || first_budget
                .maximum_input_tokens
                .checked_add(step.maximum_input_tokens)
                .is_none_or(|total| total > job.maximum_input_tokens)
            || first_budget
                .maximum_output_tokens
                .checked_add(step.maximum_output_tokens)
                .is_none_or(|total| total > job.maximum_output_tokens)
            || first_cost
                .checked_add(step_cost)
                .is_none_or(|total| total > job_cost)
            || first_wall
                .checked_add(step_wall)
                .and_then(|wall| wall.checked_add(35_000))
                .is_none_or(|wall| wall >= job_wall)
            || input
                .maker_model
                .as_ref()
                .and_then(|model| model.capabilities.as_ref())
                .is_none_or(|capabilities| !capabilities.supports_tools)
        {
            return Err(StoreError::Invalid("discovery cumulative budget"));
        }
    }
    Ok(())
}

fn validate_protocol(
    protocol: &wire::ProtocolSelectionSnapshot,
    controlled: bool,
) -> StoreResult<()> {
    let mut features = FEATURES.to_vec();
    if controlled {
        features.push("discovery.tool-context.v1");
        features.sort_unstable();
    }
    if protocol.selected_package != "loop.v1"
        || protocol
            .enabled_features
            .iter()
            .map(String::as_str)
            .ne(features)
        || protocol
            .schema_descriptor_sha256
            .as_ref()
            .is_none_or(|digest| {
                digest.value.as_slice()
                    != Sha256::digest(loop_protocol::FILE_DESCRIPTOR_SET).as_slice()
                    && digest.value.as_slice() != CONTEXT_DESCRIPTOR
                    && digest.value.as_slice() != LIFECYCLE_DESCRIPTOR
                    && digest.value.as_slice() != OPERATIONS_DESCRIPTOR
            })
        || protocol.selection_sha256.as_ref().is_none_or(|digest| {
            protocol_selection_sha256(protocol).map_or(true, |expected| digest.value != expected)
        })
    {
        return Err(StoreError::Invalid("discovery protocol pin"));
    }
    Ok(())
}

fn decode_object<T: Message + Default>(
    root: &Path,
    reference: &ObjectRef,
    files: &mut Vec<(PathBuf, ObjectRef)>,
) -> StoreResult<T> {
    let bytes = load_object(root, reference, files)?;
    let decoded =
        T::decode(bytes.as_slice()).map_err(|_| StoreError::Invalid("discovery plan protobuf"))?;
    if decoded.encode_to_vec() != bytes {
        return Err(StoreError::Invalid("discovery plan protobuf encoding"));
    }
    Ok(decoded)
}

fn load_object(
    root: &Path,
    reference: &ObjectRef,
    files: &mut Vec<(PathBuf, ObjectRef)>,
) -> StoreResult<Vec<u8>> {
    reference.digest()?;
    let path = root.join(&reference.sha256[7..]);
    let bytes = checked_read(&path, reference)?;
    files.push((path, reference.clone()));
    Ok(bytes)
}

fn checked_read(path: &Path, reference: &ObjectRef) -> StoreResult<Vec<u8>> {
    if !(1..=MAX_OBJECT).contains(&reference.byte_size) {
        return Err(StoreError::Invalid("discovery plan object size"));
    }
    let bytes = read_file(path, true, reference.byte_size)?;
    if bytes.len() as u64 != reference.byte_size
        || Sha256::digest(&bytes).as_slice() != reference.digest()?
    {
        return Err(StoreError::Corrupt("discovery plan content"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod compatibility;
#[cfg(test)]
mod tests;
