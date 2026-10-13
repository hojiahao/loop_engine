//! Private bounded adaptive state, stored only in immutable run transitions.

use loop_protocol::wire::{runs::v1 as runs, v1};
use prost::Message;
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::manifests::feedback::{DerivedFeedback, FrozenFeedback, VerifiedObservation};
use crate::manifests::perturbation::{PerturbationEvidence, PreparedPerturbation};
use crate::store::{StoreError, StoreResult};

/// Exact earlier registered portfolio and its server-resolved manifest pins.
/// Keeping the full record lets reconstruction detect later replacement or
/// cross-run injection; the model receives a separately bounded projection.
#[derive(Clone, PartialEq, Message)]
pub(crate) struct FeedbackSource {
    /// Completed round, strictly increasing within the run.
    #[prost(uint32, tag = "1")]
    pub round: u32,
    /// Immutable successful IS portfolio job.
    #[prost(message, optional, tag = "2")]
    pub record: Option<v1::JobRecord>,
    /// Exact result registered by that job's completion transaction.
    #[prost(message, optional, tag = "3")]
    pub result: Option<v1::BacktestResult>,
    /// Original WorkflowEvidence; no caller-supplied file path.
    #[prost(bytes = "vec", tag = "4")]
    pub evidence: Vec<u8>,
}

/// Successful selection consumed by the parent; its files remain required even
/// after a later stage becomes current. This list never triggers worker replay.
#[derive(Clone, PartialEq, Message)]
pub(crate) struct SelectionSource {
    /// Round which consumed this exact result.
    #[prost(uint32, tag = "1")]
    pub round: u32,
    /// Immutable successful perturbation record and its actual output references.
    #[prost(message, optional, tag = "2")]
    pub record: Option<v1::JobRecord>,
}

/// Complete context for the current proposal, frozen before reserving its job.
/// This is internal evidence, never a public RPC request or authority token.
#[derive(Clone, PartialEq, Message)]
pub(crate) struct AdaptiveBinding {
    /// Closed internal encoding, independent of the public protocol version.
    #[prost(uint32, tag = "1")]
    pub version: u32,
    /// Exact policy selected by the immutable run specification.
    #[prost(message, optional, tag = "2")]
    pub policy: Option<v1::PolicyReference>,
    /// First applicable model AST; other model proposals may remain independent.
    #[prost(message, optional, tag = "3")]
    pub family_ast: Option<v1::CanonicalFactorAst>,
    /// Semantic family identity resolved from AST, recipe and installed build.
    #[prost(message, optional, tag = "4")]
    pub family_sha256: Option<v1::Sha256Digest>,
    /// Actual installed perturbation algorithm state, never inferred from scores.
    #[prost(message, optional, tag = "5")]
    pub state: Option<v1::PerturbationState>,
    /// All successful verified portfolio observations, bounded by 64 rounds.
    #[prost(message, repeated, tag = "6")]
    pub sources: Vec<FeedbackSource>,
    /// Exact full immutable feedback source manifest, stored as canonical JSON.
    #[prost(bytes = "vec", tag = "7")]
    pub feedback_reference: Vec<u8>,
    /// Model-visible structured feedback; only the bounded suffix is exposed.
    #[prost(string, tag = "8")]
    pub feedback_text: String,
    /// Complete frozen candidate request; request ID is assigned at reservation.
    #[prost(message, optional, tag = "9")]
    pub invocation: Option<v1::ModelInvocation>,
    /// Optional first tool request sharing the same frozen feedback messages.
    #[prost(message, optional, tag = "10")]
    pub tool_invocation: Option<v1::ModelInvocation>,
    /// Exact work reserved for a perturbation proposal, retained for recovery.
    #[prost(message, optional, tag = "11")]
    pub work: Option<v1::PerturbationWork>,
    /// Last committed numerical selection result; absent before first execution.
    #[prost(message, optional, tag = "12")]
    pub step: Option<v1::PerturbationStep>,
    /// Deterministic coverage failures from this run's verified trials only.
    #[prost(message, repeated, tag = "13")]
    pub failed_factor_ids: Vec<v1::FactorSpecId>,
    /// Round whose proposal froze this context; unchanged during its evaluation.
    #[prost(uint32, tag = "14")]
    pub context_round: u32,
    /// At most one successful perturbation per noninitial round.
    #[prost(message, repeated, tag = "15")]
    pub completed_perturbations: Vec<SelectionSource>,
}

/// Verified adaptive update for one exact source child and parent revision.
/// Private fields keep caller-supplied protobuf out of the store's writer path.
pub(crate) struct AdaptiveProof {
    before: Option<AdaptiveBinding>,
    current: AdaptiveBinding,
    next: Option<AdaptiveProposal>,
    policy: Arc<FrozenFeedback>,
    sources: Vec<Arc<VerifiedObservation>>,
    original_feedback: Arc<DerivedFeedback>,
    feedback: Arc<DerivedFeedback>,
    completed: Option<Arc<PerturbationEvidence>>,
    history: Vec<Arc<PerturbationEvidence>>,
}

/// Already prepared next proposal. Reserving its budget and creating its child
/// remain one parent transaction; this object does not execute anything.
pub(crate) struct AdaptiveProposal {
    pub(crate) binding: AdaptiveBinding,
    pub(crate) job_id: String,
    pub(crate) perturbation: Option<Arc<PreparedPerturbation>>,
}

impl AdaptiveProof {
    pub(in crate::runtime) fn new(
        before: Option<AdaptiveBinding>,
        current: AdaptiveBinding,
        next: Option<AdaptiveProposal>,
        policy: Arc<FrozenFeedback>,
        sources: Vec<Arc<VerifiedObservation>>,
        original_feedback: Arc<DerivedFeedback>,
        feedback: Arc<DerivedFeedback>,
    ) -> StoreResult<Self> {
        let proof = Self {
            before,
            current,
            next,
            policy,
            sources,
            original_feedback,
            feedback,
            completed: None,
            history: vec![],
        };
        proof.check()?;
        Ok(proof)
    }

    pub(in crate::runtime) fn with_completion(
        mut self,
        completed: Arc<PerturbationEvidence>,
    ) -> StoreResult<Self> {
        self.completed = Some(completed);
        self.check()?;
        Ok(self)
    }

    pub(in crate::runtime) fn with_history(
        mut self,
        history: Vec<Arc<PerturbationEvidence>>,
    ) -> StoreResult<Self> {
        self.history = history;
        self.check_history()?;
        Ok(self)
    }

    fn check_history(&self) -> StoreResult<()> {
        if self.history.len() != self.current.completed_perturbations.len() {
            return Err(StoreError::Corrupt("adaptive selection proof count"));
        }
        for (proof, bound) in self
            .history
            .iter()
            .zip(&self.current.completed_perturbations)
        {
            proof.check_record(
                bound
                    .record
                    .as_ref()
                    .ok_or(StoreError::Corrupt("adaptive selection record"))?,
            )?;
        }
        Ok(())
    }

    pub(crate) fn check(&self) -> StoreResult<()> {
        self.policy.check()?;
        self.original_feedback.check()?;
        self.feedback.check()?;
        self.current.project()?;
        let original: crate::manifests::ObjectRef =
            serde_json::from_slice(&self.current.feedback_reference)
                .map_err(|_| StoreError::Corrupt("adaptive original feedback reference"))?;
        if &original != self.original_feedback.reference()
            || self.current.feedback_text != self.original_feedback.text()
        {
            return Err(StoreError::Corrupt("adaptive original feedback proof"));
        }
        let visible = self
            .next
            .as_ref()
            .map_or(&self.current, |next| &next.binding);
        let reference: crate::manifests::ObjectRef =
            serde_json::from_slice(&visible.feedback_reference)
                .map_err(|_| StoreError::Corrupt("adaptive feedback reference"))?;
        if &reference != self.feedback.reference()
            || visible.feedback_text != self.feedback.text()
            || self
                .current
                .policy
                .as_ref()
                .and_then(|policy| policy.sha256.as_ref())
                .map(|hash| hash.value.as_slice())
                != Some(self.policy.reference().digest()?.as_slice())
        {
            return Err(StoreError::Corrupt("adaptive feedback proof"));
        }
        if self.current.sources.len() != self.sources.len() {
            return Err(StoreError::Corrupt("adaptive source proof count"));
        }
        for (bound, proof) in self.current.sources.iter().zip(&self.sources) {
            proof.check()?;
            let (record, result) = proof.source();
            if bound.record.as_ref() != Some(record)
                || bound.result.as_ref() != Some(result)
                || bound.round != proof.entry().round
            {
                return Err(StoreError::Corrupt("adaptive source proof binding"));
            }
        }
        if let Some(next) = &self.next {
            next.binding.project()?;
            if next.binding.sources != self.current.sources
                || next.binding.state != self.current.state
                || next.binding.family_ast != self.current.family_ast
                || next.binding.family_sha256 != self.current.family_sha256
                || next.binding.failed_factor_ids != self.current.failed_factor_ids
                || next.binding.completed_perturbations != self.current.completed_perturbations
            {
                return Err(StoreError::Corrupt("adaptive proposal state"));
            }
            if let Some(prepared) = &next.perturbation {
                prepared.check()?;
                if next.binding.work.as_ref() != Some(prepared.work()) {
                    return Err(StoreError::Corrupt("adaptive work proof"));
                }
            } else if next.binding.work.is_some() {
                return Err(StoreError::Corrupt("adaptive model work"));
            }
        }
        if let Some(completed) = &self.completed {
            completed.check()?;
            if self.current.step.as_ref() != Some(completed.step())
                || self.current.state != completed.step().state
            {
                return Err(StoreError::Corrupt("adaptive result proof"));
            }
        }
        Ok(())
    }

    pub(crate) fn check_for(&self, run: &runs::RunSpecification) -> StoreResult<()> {
        self.check()?;
        if self.current.policy != run.feedback_policy
            || self.policy.schedule_len() != run.maximum_rounds as usize
            || self.current.sources.iter().any(|source| {
                source
                    .record
                    .as_ref()
                    .and_then(|record| record.specification.as_ref())
                    .is_none_or(|job| job.run_id != run.run_id)
            })
        {
            return Err(StoreError::AdmissionDenied);
        }
        self.check_history()?;
        Ok(())
    }

    pub(crate) fn before(&self) -> Option<&AdaptiveBinding> {
        self.before.as_ref()
    }
    pub(crate) fn current(&self) -> &AdaptiveBinding {
        &self.current
    }
    pub(crate) fn next(&self) -> Option<&AdaptiveProposal> {
        self.next.as_ref()
    }
    pub(crate) fn completion(&self) -> Option<&Arc<PerturbationEvidence>> {
        self.completed.as_ref()
    }
}

impl AdaptiveBinding {
    pub(crate) fn project(&self) -> StoreResult<runs::AdaptiveRunView> {
        if self.version != 1
            || !(1..=64).contains(&self.context_round)
            || self.policy.is_none()
            || self.sources.len() > 64
            || self.failed_factor_ids.len() > 64
            || self.completed_perturbations.len() > 63
            || self.feedback_text.len() > 16_384
            || self.feedback_reference.is_empty()
            || self.family_ast.is_some() != self.family_sha256.is_some()
            || self.family_ast.is_some() != self.state.is_some()
        {
            return Err(StoreError::Corrupt("adaptive state shape"));
        }
        let reference: crate::manifests::ObjectRef =
            serde_json::from_slice(&self.feedback_reference)
                .map_err(|_| StoreError::Corrupt("adaptive feedback reference"))?;
        reference.digest()?;
        let invocation = self
            .invocation
            .as_ref()
            .ok_or(StoreError::Corrupt("adaptive model context"))?;
        if invocation.request_id.is_some()
            || self
                .tool_invocation
                .as_ref()
                .is_some_and(|invocation| invocation.request_id.is_some())
        {
            return Err(StoreError::Corrupt("adaptive request identity"));
        }
        let mut context = Sha256::new();
        context.update(b"loop.adaptive-model-context/v1\0");
        let bytes = invocation.encode_to_vec();
        context.update((bytes.len() as u64).to_be_bytes());
        context.update(bytes);
        if let Some(first) = &self.tool_invocation {
            let bytes = first.encode_to_vec();
            context.update((bytes.len() as u64).to_be_bytes());
            context.update(bytes);
        }
        Ok(runs::AdaptiveRunView {
            feedback_policy: self.policy.clone(),
            source_count: selected_count(&self.feedback_text)?,
            feedback_sha256: Some(v1::Sha256Digest {
                value: reference.digest()?.to_vec(),
            }),
            model_context_sha256: Some(v1::Sha256Digest {
                value: context.finalize().to_vec(),
            }),
            family_sha256: self.family_sha256.clone(),
            random_draws: self.state.as_ref().map_or(0, |state| state.random_draws),
            state_sha256: self.state.as_ref().map(|state| v1::Sha256Digest {
                value: Sha256::digest(state.encode_to_vec()).to_vec(),
            }),
        })
    }
}

fn selected_count(text: &str) -> StoreResult<u32> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| StoreError::Corrupt("adaptive feedback JSON"))?;
    let sources = value
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .ok_or(StoreError::Corrupt("adaptive feedback sources"))?;
    if sources.len() > 16 {
        return Err(StoreError::Corrupt("adaptive feedback bounds"));
    }
    Ok(sources.len() as u32)
}
