//! Frozen feedback limits and source-backed, bounded research observations.
//! These file proofs do not establish database registration or caller authority.

use std::sync::Arc;

use loop_core::factor::{PolicyId, PositiveInteger};
use loop_protocol::wire::v1 as wire;
use serde::{Deserialize, Serialize};

use super::files::{ReadBudget, VerifiedFile};
use super::numerical::NumericalResolver;
use super::policy::check_files;
use super::{ObjectRef, model};
use crate::store::{StoreError, StoreResult};

mod observation;
pub(crate) use observation::{ObservationSource, VerifiedObservation};

/// A predeclared proposal mode, never selected by a model response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Proposal {
    Discovery,
    Perturbation,
}

/// Closed administrator policy. Its order is the canonical JSON field order.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FeedbackPolicy {
    pub schema: String,
    pub id: String,
    pub revision: String,
    pub proposal_schedule: Vec<Proposal>,
    pub window_path: Vec<u32>,
    pub eligible_windows: Vec<u32>,
    pub random_seed: String,
    pub max_observations: u32,
    pub max_bytes: u32,
}

/// Validated policy bytes, retained across final transaction guards.
#[derive(Clone)]
pub(crate) struct FrozenFeedback {
    reference: ObjectRef,
    policy: FeedbackPolicy,
    files: Vec<Arc<VerifiedFile>>,
}

/// Immutable full source manifest and bounded model-visible text. Full sources
/// are retained for replay; truncation affects only the explicitly bounded view.
pub(crate) struct DerivedFeedback {
    reference: ObjectRef,
    text: String,
    source_count: usize,
    sources: Vec<Arc<VerifiedObservation>>,
    files: Vec<Arc<VerifiedFile>>,
}

#[derive(Serialize)]
struct FeedbackDocument<'a> {
    schema: &'static str,
    policy: &'a ObjectRef,
    run_id: &'a str,
    round: u32,
    sources: Vec<&'a observation::FeedbackEntry>,
}

impl NumericalResolver {
    /// Publish deterministic evidence before freezing the next model request.
    /// On replay, missing objects are errors and are never silently recreated.
    pub(crate) async fn feedback_context(
        &self,
        policy: &FrozenFeedback,
        run_id: &str,
        round: u32,
        sources: Vec<Arc<VerifiedObservation>>,
        replay: bool,
    ) -> StoreResult<DerivedFeedback> {
        policy.check()?;
        crate::store::validate_id(run_id)?;
        if sources.len() > 64 || !(1..=64).contains(&round) {
            return Err(StoreError::Invalid("feedback source bounds"));
        }
        let mut jobs = std::collections::BTreeSet::new();
        let mut previous = 0;
        for source in &sources {
            source.check()?;
            let job = source
                .source()
                .0
                .specification
                .as_ref()
                .ok_or(StoreError::Corrupt("feedback source"))?;
            if job.run_id.as_ref().map(|id| id.value.as_str()) != Some(run_id)
                || source.entry.round <= previous
                || source.entry.round >= round
                || !jobs.insert(&source.entry.job_id)
            {
                return Err(StoreError::Corrupt("feedback source order"));
            }
            previous = source.entry.round;
        }
        let document = FeedbackDocument {
            schema: "loop.run-feedback-sources/v1",
            policy: policy.reference(),
            run_id,
            round,
            sources: sources.iter().map(|source| source.entry()).collect(),
        };
        let selected = sources
            .len()
            .saturating_sub(policy.policy.max_observations as usize);
        let text = serde_json::to_string(&FeedbackDocument {
            sources: document.sources[selected..].to_vec(),
            ..document
        })
        .map_err(|_| StoreError::Corrupt("feedback encoding"))?;
        if text.len() > policy.policy.max_bytes as usize {
            return Err(StoreError::Invalid("feedback message bound"));
        }
        let document = FeedbackDocument {
            schema: "loop.run-feedback-sources/v1",
            policy: policy.reference(),
            run_id,
            round,
            sources: sources.iter().map(|source| source.entry()).collect(),
        };
        let mut files = policy.guards();
        let reference = self.json(&document, replay, &mut files).await?;
        let derived = DerivedFeedback {
            reference,
            text,
            source_count: sources.len() - selected,
            sources,
            files,
        };
        derived.check()?;
        Ok(derived)
    }

    /// Resolve the exact plan-store object, without accepting mutable settings.
    pub(crate) async fn feedback(
        &self,
        policy: &wire::PolicyReference,
        reference: &ObjectRef,
    ) -> StoreResult<FrozenFeedback> {
        self.check_root()?;
        let file = self
            .plans
            .load(reference, true, &mut ReadBudget::new())
            .await?;
        let document: FeedbackPolicy = file.json()?;
        document.validate()?;
        if policy.policy_id.as_ref().map(|id| id.value.as_str()) != Some(&document.id)
            || policy.revision != document.revision
            || policy.sha256.as_ref().map(|hash| hash.value.as_slice())
                != Some(reference.digest()?.as_slice())
        {
            return Err(StoreError::Corrupt("feedback policy identity"));
        }
        file.check()?;
        Ok(FrozenFeedback {
            reference: reference.clone(),
            policy: document,
            files: vec![file],
        })
    }
}

impl DerivedFeedback {
    pub(crate) fn reference(&self) -> &ObjectRef {
        &self.reference
    }
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
    pub(crate) fn source_count(&self) -> usize {
        self.source_count
    }

    pub(crate) fn check(&self) -> StoreResult<()> {
        for source in &self.sources {
            source.check()?;
        }
        check_files(&self.files)
    }
}

impl FeedbackPolicy {
    fn validate(&self) -> StoreResult<()> {
        model::schema(&self.schema, "loop.run-feedback/v1")?;
        PolicyId::new(&self.id).map_err(|_| StoreError::Invalid("feedback policy ID"))?;
        PositiveInteger::new(&self.revision)
            .map_err(|_| StoreError::Invalid("feedback revision"))?;
        model::parse_digest(&self.random_seed)?;
        if !(1..=64).contains(&self.proposal_schedule.len())
            || self.proposal_schedule.first() != Some(&Proposal::Discovery)
            || !(1..=32).contains(&self.window_path.len())
            || self.window_path.iter().any(|index| *index >= 1024)
            || !(2..=64).contains(&self.eligible_windows.len())
            || self
                .eligible_windows
                .iter()
                .any(|window| !(1..=4096).contains(window))
            || self
                .eligible_windows
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || !(1..=16).contains(&self.max_observations)
            || !(1..=16_384).contains(&self.max_bytes)
        {
            return Err(StoreError::Invalid("feedback policy bounds"));
        }
        Ok(())
    }
}

impl FrozenFeedback {
    pub(crate) fn check(&self) -> StoreResult<()> {
        check_files(&self.files)
    }

    pub(crate) fn reference(&self) -> &ObjectRef {
        &self.reference
    }

    pub(crate) fn policy(&self) -> &FeedbackPolicy {
        &self.policy
    }

    /// One-based round index. Exhausting a finite schedule is not implicit reuse.
    pub(crate) fn proposal(&self, round: u32) -> StoreResult<Proposal> {
        self.check()?;
        round
            .checked_sub(1)
            .and_then(|index| self.policy.proposal_schedule.get(index as usize))
            .copied()
            .ok_or(StoreError::Invalid("feedback schedule exhausted"))
    }

    pub(crate) fn schedule_len(&self) -> usize {
        self.policy.proposal_schedule.len()
    }

    pub(super) fn guards(&self) -> Vec<Arc<VerifiedFile>> {
        self.files.clone()
    }
}

#[cfg(test)]
mod tests;
