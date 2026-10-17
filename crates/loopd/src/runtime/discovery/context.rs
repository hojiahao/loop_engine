//! Server-derived, job-scoped model context. No public request supplies messages.

use std::sync::Arc;

use loop_protocol::wire::{runs::v1::RunSpecification, v1};
use prost::Message;
use serde::Deserialize;

use super::{DiscoveryExecutor, plan::FrozenPlan};
use crate::manifests::{ObjectRef, feedback::DerivedFeedback};
use crate::runtime::{model_codec, workflow::adaptive::AdaptiveBinding};
use crate::store::{StoreError, StoreResult};

/// Exact full templates frozen before job insertion or any provider reservation.
/// Only validated source-backed feedback can construct this value.
pub(crate) struct FrozenContext {
    invocation: v1::ModelInvocation,
    tool_invocation: Option<v1::ModelInvocation>,
}

impl FrozenContext {
    pub(crate) fn invocation(&self) -> &v1::ModelInvocation {
        &self.invocation
    }
    pub(crate) fn tool_invocation(&self) -> Option<&v1::ModelInvocation> {
        self.tool_invocation.as_ref()
    }
}

pub(super) struct ContextScope {
    job: v1::JobSpecification,
    feedback: Arc<DerivedFeedback>,
}

impl ContextScope {
    pub(super) fn check(&self, job: &v1::JobSpecification) -> StoreResult<()> {
        if job != &self.job {
            return Err(StoreError::AdmissionDenied);
        }
        self.feedback.check()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeedbackIdentity {
    schema: String,
    policy: ObjectRef,
    run_id: String,
    round: u32,
    sources: Vec<serde_json::Value>,
}

impl DiscoveryExecutor {
    /// Derive a bounded context from installed static templates and verified IS
    /// feedback. Budgets, models, tools and provider policies remain frozen.
    pub(crate) fn freeze_context(
        &self,
        specification: &RunSpecification,
        feedback: &DerivedFeedback,
    ) -> StoreResult<FrozenContext> {
        if self.context_scope.is_some() || specification.numerical.is_none() {
            return Err(StoreError::AdmissionDenied);
        }
        self.verify_run(specification)?;
        let plan = self
            .plans
            .iter()
            .find(|plan| Some(&plan.input) == specification.discovery.as_ref())
            .ok_or(StoreError::AdmissionDenied)?;
        let policy = specification
            .feedback_policy
            .as_ref()
            .ok_or(StoreError::AdmissionDenied)?;
        frozen_context(plan, policy, feedback)
    }

    /// Reconstruct the exact frozen templates, never rebuild from mutable text
    /// or accept a serialized invocation as authority. The returned executor is
    /// restricted to this immutable job and retains all source file guards.
    pub(crate) fn scope_existing(
        &self,
        job: &v1::JobSpecification,
        binding: &AdaptiveBinding,
        feedback: Arc<DerivedFeedback>,
    ) -> StoreResult<Self> {
        if self.context_scope.is_some() || binding.version != 1 {
            return Err(StoreError::AdmissionDenied);
        }
        let plan = self.plan(job)?;
        let policy = binding.policy.as_ref().ok_or(StoreError::AdmissionDenied)?;
        let expected = frozen_context(plan, policy, &feedback)?;
        let reference = serde_json::to_vec(feedback.reference())
            .map_err(|_| StoreError::Corrupt("adaptive feedback reference"))?;
        if binding.feedback_reference != reference
            || binding.feedback_text != feedback.text()
            || binding.invocation.as_ref() != Some(expected.invocation())
            || binding.tool_invocation.as_ref() != expected.tool_invocation()
        {
            return Err(StoreError::Corrupt("adaptive frozen context"));
        }
        let scoped = plan.with_context(
            expected.invocation,
            expected.tool_invocation,
            Arc::clone(&feedback),
        )?;
        Ok(Self {
            plans: vec![scoped],
            data: Arc::clone(&self.data),
            provider: self.provider.clone(),
            context_scope: Some(ContextScope {
                job: job.clone(),
                feedback,
            }),
        })
    }
}

fn frozen_context(
    plan: &FrozenPlan,
    policy: &v1::PolicyReference,
    feedback: &DerivedFeedback,
) -> StoreResult<FrozenContext> {
    plan.check()?;
    feedback.check()?;
    if feedback.text().is_empty() || feedback.text().len() > 16_384 || feedback.source_count() > 16
    {
        return Err(StoreError::Invalid("adaptive feedback bound"));
    }
    let identity: FeedbackIdentity = serde_json::from_str(feedback.text())
        .map_err(|_| StoreError::Corrupt("adaptive feedback identity"))?;
    if identity.schema != "loop.run-feedback-sources/v1"
        || identity.run_id != plan.run_id
        || !(1..=64).contains(&identity.round)
        || identity.sources.len() != feedback.source_count()
        || policy.sha256.as_ref().map(|digest| digest.value.as_slice())
            != Some(identity.policy.digest()?.as_slice())
    {
        return Err(StoreError::AdmissionDenied);
    }
    let mut invocation = plan.invocation.clone();
    append_feedback(&mut invocation, feedback.text())?;
    let tool_invocation = plan
        .tool_invocation
        .clone()
        .map(|mut first| -> StoreResult<_> {
            append_feedback(&mut first, feedback.text())?;
            Ok(first)
        })
        .transpose()?;
    if let Some(first) = &tool_invocation {
        model_codec::validate_tools(first, false)?;
        model_codec::validate_tools(&invocation, true)?;
        if first.messages != invocation.messages
            || first.messages.len() > 30
            || first
                .messages
                .iter()
                .map(Message::encoded_len)
                .sum::<usize>()
                > 112_640
        {
            return Err(StoreError::Invalid("adaptive tool context bound"));
        }
    } else {
        model_codec::validate_template(&invocation)?;
    }
    Ok(FrozenContext {
        invocation,
        tool_invocation,
    })
}

fn append_feedback(invocation: &mut v1::ModelInvocation, text: &str) -> StoreResult<()> {
    if invocation.request_id.is_some() {
        return Err(StoreError::Corrupt("adaptive request identity"));
    }
    invocation.messages.push(v1::ModelMessage {
        role: v1::ModelRole::User as i32,
        content: vec![v1::ContentBlock {
            content: Some(v1::content_block::Content::Text(v1::TextContent {
                text: text.to_owned(),
            })),
        }],
    });
    model_codec::validate_context(&invocation.messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation() -> v1::ModelInvocation {
        v1::ModelInvocation {
            model: Some(v1::ModelResolutionSnapshot::default()),
            budget: Some(v1::InvocationBudget {
                maximum_input_tokens: 4096,
                ..Default::default()
            }),
            request_policy: Some(v1::PolicyReference::default()),
            messages: vec![v1::ModelMessage {
                role: v1::ModelRole::User as i32,
                content: vec![v1::ContentBlock {
                    content: Some(v1::content_block::Content::Text(v1::TextContent {
                        text: "Propose one factor.".into(),
                    })),
                }],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn context_append() {
        let original = invocation();
        let mut actual = original.clone();
        append_feedback(&mut actual, "{\"sources\":[]}").unwrap();
        assert_eq!(actual.model, original.model);
        assert_eq!(actual.budget, original.budget);
        assert_eq!(actual.request_policy, original.request_policy);
        assert_eq!(actual.messages[0], original.messages[0]);
        assert_eq!(actual.messages[1].role, v1::ModelRole::User as i32);
        actual.messages.pop();
        assert_eq!(actual, original);
    }

    #[test]
    fn context_bounds() {
        let mut actual = invocation();
        actual.messages.resize(32, actual.messages[0].clone());
        assert!(append_feedback(&mut actual, "{}").is_err());
        let mut actual = invocation();
        assert!(append_feedback(&mut actual, &"x".repeat(131_072)).is_err());
    }

    #[test]
    fn reserved_context() {
        let mut actual = invocation();
        actual.request_id = Some(v1::RequestId {
            value: "reserved".into(),
        });
        let original = actual.clone();
        assert!(append_feedback(&mut actual, "{}").is_err());
        assert_eq!(actual, original);
    }
}
