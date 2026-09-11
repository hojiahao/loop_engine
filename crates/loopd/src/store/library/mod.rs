//! One admission/readmission path over committed development research evidence.

mod storage;
pub(super) mod trials;
mod types;

use std::future::Future;
use std::time::Duration;

use loop_core::audit::{AuditAction, AuditTarget, AuditTargetKind, canonicalize_audit_payload};
use loop_core::factor::FactorSpecId;
use loop_protocol::artifact::validate_artifact_ref;
use loop_protocol::wire::v1::{Actor, ActorKind, BacktestEngineKind, job_specification};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use super::lifecycle::validate_context;
use super::postgres::{record_from_row, timestamp, timestamp_millis};
use super::{PgJobStore, StoreError, StoreResult, audit, backtest, validate_id};
use types::Receipt;
pub use types::{AdmissionEvidence, DecideFactor, FactorDecision, FactorState, FactorTrial};

const OPERATION: &str = "loop.factors.decide";

/// Backend-independent factor decisions and bounded trial accounting.
pub trait FactorRepository: Send + Sync {
    /// Use the same mandatory gates for first consideration and readmission.
    /// The principal comes from transport authentication. Evidence is resolved
    /// by the service, not supplied metrics. Override can waive only a semantic
    /// rejection with a verified human approval. State, replacements, receipt
    /// and audit commit atomically. Replays reauthorize and check current source
    /// provenance but do not repeat effects. Invalid/denied/stale evidence,
    /// conflicts, corruption and outages are errors, never factor rejection.
    /// Cancellation rolls back before commit; retry resolves uncertain commits.
    fn decide_factor(
        &self,
        principal: &Actor,
        command: DecideFactor,
    ) -> impl Future<Output = StoreResult<FactorDecision>> + Send;

    /// Page registered trials in one run by exclusive job-ID cursor, at most
    /// 500 per call. Authorize every returned job and verify its immutable input
    /// binding. Outcomes are current job projections, not admission votes. Empty
    /// runs expose no existence information. Denied/corrupt pages return nothing;
    /// the read has no persistent effects and cancellation aborts it.
    fn factor_trials(
        &self,
        principal: &Actor,
        run_id: &str,
        after: &str,
        limit: u32,
    ) -> impl Future<Output = StoreResult<Vec<FactorTrial>>> + Send;
}

impl FactorRepository for PgJobStore {
    async fn decide_factor(
        &self,
        principal: &Actor,
        command: DecideFactor,
    ) -> StoreResult<FactorDecision> {
        tokio::time::timeout(Duration::from_secs(30), execute(self, principal, command))
            .await
            .map_err(|_| StoreError::Unavailable("factor decision timeout"))?
    }

    async fn factor_trials(
        &self,
        principal: &Actor,
        run_id: &str,
        after: &str,
        limit: u32,
    ) -> StoreResult<Vec<FactorTrial>> {
        tokio::time::timeout(
            Duration::from_secs(30),
            trials::page(self, principal, run_id, after, limit),
        )
        .await
        .map_err(|_| StoreError::Unavailable("trial read timeout"))?
    }
}

impl DecideFactor {
    fn normalized(&self) -> Self {
        let mut normalized = self.clone();
        if let Some(context) = &mut normalized.context {
            context.request_id = None;
            context.requested_at = None;
        }
        normalized.deadline = None;
        normalized
    }
}

async fn execute(
    store: &PgJobStore,
    principal: &Actor,
    command: DecideFactor,
) -> StoreResult<FactorDecision> {
    let context = validate_context(command.context.as_ref(), principal)?;
    let job_id = &command
        .source_job_id
        .as_ref()
        .ok_or(StoreError::Invalid("factor source job"))?
        .value;
    validate_id(job_id)?;
    validate_id(&command.context_id)?;
    text(&command.reason)?;
    let force = !command.override_reason.is_empty();
    if force == command.override_approval_id.is_empty()
        || command.expected_revision >= i64::MAX as u64
    {
        return Err(StoreError::Invalid("factor override or revision"));
    }
    if force {
        text(&command.override_reason)?;
        validate_id(&command.override_approval_id)?;
        if principal.kind != ActorKind::Human as i32 {
            return Err(StoreError::AdmissionDenied);
        }
    }
    let requested = timestamp_millis(
        context
            .requested_at
            .as_ref()
            .ok_or(StoreError::Invalid("factor request time"))?,
        true,
    )?;
    let deadline = timestamp_millis(
        command
            .deadline
            .as_ref()
            .ok_or(StoreError::Invalid("factor deadline"))?,
        true,
    )?;
    if deadline <= requested || deadline - requested > 30_000 {
        return Err(StoreError::Invalid("factor deadline"));
    }
    let prepared_store = store
        .prepare_research(
            principal,
            job_id,
            Some(&command.context_id),
            OPERATION,
            None,
        )
        .await?;
    let store = &prepared_store;
    let mut transaction = store.pool.begin().await?;
    let now = store.observe_clock(&mut transaction).await?;
    check_time(now, requested, deadline)?;
    // Exclude protected inputs before calling any result/holdout materializer.
    let row = sqlx::query("SELECT * FROM jobs WHERE job_id = $1")
        .bind(job_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StoreError::NotFound)?;
    let source = record_from_row(&row)?;
    store
        .admission
        .authorize_job_command(OPERATION, principal, &source)?;
    let specification = source
        .specification
        .as_ref()
        .ok_or(StoreError::Corrupt("factor source"))?;
    if !matches!(
        specification.input,
        Some(job_specification::Input::Backtest(_))
    ) {
        return Err(StoreError::AdmissionDenied);
    }
    let trial = trials::verify(&mut transaction, &source).await?;
    let (_, result) = backtest::current_in_transaction(
        store,
        &mut transaction,
        principal,
        job_id,
        &command.context_id,
        OPERATION,
    )
    .await?;
    if result.engine != BacktestEngineKind::PrimaryCrossSectional as i32 {
        return Err(StoreError::Invalid("primary IS result required"));
    }
    let evidence =
        store
            .backtest_policy
            .resolve_admission(principal, specification, &command.context_id)?;
    validate_evidence(
        &evidence,
        result
            .result_manifest_sha256
            .as_ref()
            .map(|d| d.value.as_slice()),
    )?;
    if force {
        store.admission.authorize_job_command(
            "loop.factors.override_semantic",
            principal,
            &source,
        )?;
        store
            .backtest_policy
            .authorize_factor_override(principal, &command, &evidence)?;
    }
    if let Some(receipt) = storage::load_receipt(&mut transaction, &command).await? {
        if receipt.evidence.as_ref() != Some(&evidence)
            || receipt.principal.as_ref() != Some(principal)
        {
            return Err(StoreError::Corrupt("factor replay evidence"));
        }
        validate_receipt(&receipt)?;
        if receipt.states[0].factor_spec_id != trial.factor_spec_id
            || timestamp_millis(
                receipt
                    .accepted_at
                    .as_ref()
                    .ok_or(StoreError::Corrupt("factor time"))?,
                false,
            )? > now
        {
            return Err(StoreError::Corrupt("factor replay source or time"));
        }
        check_time(store.clock.now_millis()?, now, deadline)?;
        transaction.commit().await?;
        return decision(receipt, true);
    }
    let previous =
        storage::state(&mut transaction, &command.context_id, &trial.factor_spec_id).await?;
    if previous.as_ref().map_or(0, |state| state.revision) != command.expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    if previous.as_ref().is_some_and(|state| {
        state.status == "admitted" || (state.source_job_id == *job_id && !force)
    }) {
        return Err(StoreError::InvalidTransition);
    }
    let active = storage::active(&mut transaction, &command.context_id).await?;
    if active_digest(&active)? != evidence.library_sha256.as_slice() {
        return Err(StoreError::RevisionConflict);
    }
    let rejection = rejection_code(&evidence).to_owned();
    if force && rejection != "semantic_review" {
        return Err(StoreError::Invalid(
            "only semantic rejection is overridable",
        ));
    }
    let admitted = rejection.is_empty() || force;
    if admitted && active.len() >= 4096 && evidence.replacements.is_empty() {
        return Err(StoreError::Unavailable("active library capacity"));
    }
    let mut previous_states: Vec<_> = previous.iter().cloned().collect();
    let mut state = previous.unwrap_or(FactorState {
        factor_spec_id: trial.factor_spec_id,
        revision: 0,
        status: String::new(),
        admissions: 0,
        retirements: 0,
        source_job_id: job_id.clone(),
    });
    state.revision = increment(state.revision)?;
    state.status = if admitted { "admitted" } else { "rejected" }.to_owned();
    state.source_job_id = job_id.clone();
    if admitted {
        state.admissions = increment(state.admissions)?;
    }
    let mut states = vec![state];
    if admitted {
        for factor in &evidence.replacements {
            let mut retired = active
                .iter()
                .find(|state| state.factor_spec_id == *factor)
                .ok_or(StoreError::Invalid("replacement must be active"))?
                .clone();
            if retired.factor_spec_id == states[0].factor_spec_id {
                return Err(StoreError::Invalid("self replacement"));
            }
            previous_states.push(retired.clone());
            retired.revision = increment(retired.revision)?;
            retired.retirements = increment(retired.retirements)?;
            retired.status = "retired".to_owned();
            states.push(retired);
        }
    }
    let committed = store.clock.now_millis()?;
    check_time(committed, now, deadline)?;
    let receipt = Receipt {
        command: Some(command),
        evidence: Some(evidence),
        states,
        rejection_code: if admitted { String::new() } else { rejection },
        override_applied: force,
        accepted_at: Some(timestamp(committed)),
        principal: Some(principal.clone()),
        previous_states,
    };
    validate_receipt(&receipt)?;
    storage::write_states(&mut transaction, &receipt).await?;
    storage::save_receipt(&mut transaction, &receipt).await?;
    #[cfg(test)]
    super::crash_tests::fault_point("factor_after_state").await;
    append_events(store, &mut transaction, &receipt, committed).await?;
    #[cfg(test)]
    super::crash_tests::fault_point("factor_before_commit").await;
    check_time(store.clock.now_millis()?, committed, deadline)?;
    transaction.commit().await?;
    #[cfg(test)]
    super::crash_tests::fault_point("factor_after_commit").await;
    decision(receipt, false)
}

fn validate_evidence(evidence: &AdmissionEvidence, manifest: Option<&[u8]>) -> StoreResult<()> {
    let report = evidence
        .report
        .as_ref()
        .ok_or(StoreError::Unavailable("admission report"))?;
    validate_artifact_ref(report).map_err(|_| StoreError::Invalid("admission artifact"))?;
    let policy = evidence
        .policy
        .as_ref()
        .ok_or(StoreError::Unavailable("admission policy"))?;
    validate_id(
        &policy
            .policy_id
            .as_ref()
            .ok_or(StoreError::Invalid("admission policy id"))?
            .value,
    )?;
    let revision = policy
        .revision
        .parse::<u64>()
        .map_err(|_| StoreError::Invalid("admission policy revision"))?;
    if revision == 0
        || revision.to_string() != policy.revision
        || policy.sha256.as_ref().is_none_or(|d| d.value.len() != 32)
        || evidence.result_manifest_sha256.len() != 32
        || Some(evidence.result_manifest_sha256.as_slice()) != manifest
        || evidence.library_sha256.len() != 32
        || evidence.eligible_observations == 0
        || evidence.valid_observations > evidence.eligible_observations
        || !(1..=10000).contains(&evidence.minimum_coverage_bps)
        || !matches!(
            evidence.machine_rejection.as_str(),
            "" | "deterministic_filter" | "performance" | "correlation" | "policy"
        )
        || evidence.replacements.len() > 16
        || evidence
            .replacements
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(StoreError::Invalid("admission evidence"));
    }
    for factor in &evidence.replacements {
        FactorSpecId::parse(factor).map_err(|_| StoreError::Invalid("replacement identity"))?;
    }
    Ok(())
}

fn rejection_code(evidence: &AdmissionEvidence) -> &str {
    if u128::from(evidence.valid_observations) * 10000
        < u128::from(evidence.eligible_observations) * u128::from(evidence.minimum_coverage_bps)
    {
        "insufficient_coverage"
    } else if !evidence.machine_rejection.is_empty() {
        &evidence.machine_rejection
    } else if !evidence.semantic_accepted {
        "semantic_review"
    } else {
        ""
    }
}

pub(super) fn active_digest(states: &[FactorState]) -> StoreResult<[u8; 32]> {
    let entries: Vec<_> = states
        .iter()
        .map(|state| (&state.factor_spec_id, state.revision))
        .collect();
    let mut digest = Sha256::new();
    digest.update(b"loop.active-library.v1\0");
    digest
        .update(serde_json::to_vec(&entries).map_err(|_| StoreError::Invalid("library snapshot"))?);
    Ok(digest.finalize().into())
}

fn increment(value: u64) -> StoreResult<u64> {
    value
        .checked_add(1)
        .filter(|next| *next <= i64::MAX as u64)
        .ok_or(StoreError::Unavailable("factor counter exhausted"))
}

fn text(value: &str) -> StoreResult<()> {
    if value.trim().is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(StoreError::Invalid("factor reason"));
    }
    Ok(())
}

fn check_time(now: i64, previous: i64, deadline: i64) -> StoreResult<()> {
    if now < previous {
        return Err(StoreError::ClockRegression);
    }
    if now >= deadline {
        return Err(StoreError::Unavailable("factor deadline exceeded"));
    }
    Ok(())
}

fn decision(receipt: Receipt, replayed: bool) -> StoreResult<FactorDecision> {
    if receipt.states.is_empty() || receipt.states.len() > 17 {
        return Err(StoreError::Corrupt("factor decision states"));
    }
    Ok(FactorDecision {
        states: receipt.states,
        rejection_code: receipt.rejection_code,
        override_applied: receipt.override_applied,
        accepted_at: receipt
            .accepted_at
            .ok_or(StoreError::Corrupt("factor decision time"))?,
        replayed,
    })
}

fn validate_receipt(receipt: &Receipt) -> StoreResult<()> {
    let command = receipt
        .command
        .as_ref()
        .ok_or(StoreError::Corrupt("factor command"))?;
    let evidence = receipt
        .evidence
        .as_ref()
        .ok_or(StoreError::Corrupt("factor evidence"))?;
    let first = receipt
        .states
        .first()
        .ok_or(StoreError::Corrupt("factor states"))?;
    let force = !command.override_reason.is_empty();
    let rejection = rejection_code(evidence);
    let admitted = rejection.is_empty() || force;
    for state in receipt.previous_states.iter().chain(&receipt.states) {
        FactorSpecId::parse(&state.factor_spec_id)
            .map_err(|_| StoreError::Corrupt("factor state id"))?;
        validate_id(&state.source_job_id).map_err(|_| StoreError::Corrupt("factor source id"))?;
        if state.revision == 0
            || state.revision > i64::MAX as u64
            || state.admissions > state.revision
            || state.retirements > state.revision
            || !matches!(state.status.as_str(), "admitted" | "rejected" | "retired")
            || state.admissions != state.retirements + u64::from(state.status == "admitted")
        {
            return Err(StoreError::Corrupt("factor state counters"));
        }
    }
    let prior = receipt
        .previous_states
        .iter()
        .find(|s| s.factor_spec_id == first.factor_spec_id);
    let previous_revision = prior.map_or(0, |s| s.revision);
    if receipt.states.len() > 17
        || receipt.previous_states.len() > 17
        || receipt.previous_states.len() != usize::from(prior.is_some()) + receipt.states.len() - 1
        || previous_revision != command.expected_revision
        || first.revision != previous_revision + 1
        || prior.is_some_and(|s| s.status == "admitted")
        || first.status != if admitted { "admitted" } else { "rejected" }
        || first.admissions != prior.map_or(0, |s| s.admissions) + u64::from(admitted)
        || first.retirements != prior.map_or(0, |s| s.retirements)
        || Some(first.source_job_id.as_str())
            != command.source_job_id.as_ref().map(|j| j.value.as_str())
        || receipt.override_applied != force
        || (force && rejection != "semantic_review")
        || receipt.rejection_code != if admitted { "" } else { rejection }
        || receipt.states.len()
            != 1 + if admitted {
                evidence.replacements.len()
            } else {
                0
            }
        || receipt.principal.as_ref() != command.context.as_ref().and_then(|c| c.actor.as_ref())
    {
        return Err(StoreError::Corrupt("factor decision transition"));
    }
    for (state, replacement) in receipt.states[1..].iter().zip(&evidence.replacements) {
        let old = receipt
            .previous_states
            .iter()
            .find(|old| old.factor_spec_id == *replacement)
            .ok_or(StoreError::Corrupt("retirement prior state"))?;
        if state.factor_spec_id != *replacement
            || old.status != "admitted"
            || state.status != "retired"
            || state.factor_spec_id == first.factor_spec_id
            || state.revision != old.revision + 1
            || state.admissions != old.admissions
            || state.retirements != old.retirements + 1
            || state.source_job_id != old.source_job_id
        {
            return Err(StoreError::Corrupt("retirement transition"));
        }
    }
    Ok(())
}

async fn append_events(
    store: &PgJobStore,
    transaction: &mut Transaction<'_, Postgres>,
    receipt: &Receipt,
    now: i64,
) -> StoreResult<()> {
    let command = receipt
        .command
        .as_ref()
        .ok_or(StoreError::Invalid("factor command"))?;
    let context = command
        .context
        .as_ref()
        .ok_or(StoreError::Invalid("factor context"))?;
    let principal = receipt
        .principal
        .as_ref()
        .ok_or(StoreError::AdmissionDenied)?;
    let factor = &receipt.states[0].factor_spec_id;
    let evidence = receipt
        .evidence
        .as_ref()
        .and_then(|e| e.report.as_ref())
        .and_then(|r| r.artifact_id.as_ref())
        .ok_or(StoreError::Invalid("factor evidence"))?;
    let mut events = Vec::new();
    if receipt.override_applied {
        events.push((
            AuditAction::OverrideAuthorized,
            factor.clone(),
            event_payload(
                "loop.audit.override_authorized",
                &[
                    ("factor_spec_id", factor),
                    ("override_kind", "force_admission"),
                    (
                        "authorized_by_actor_id",
                        &principal
                            .actor_id
                            .as_ref()
                            .ok_or(StoreError::AdmissionDenied)?
                            .value,
                    ),
                    ("reason", &command.override_reason),
                    ("approval_reference", &command.override_approval_id),
                    ("evidence_artifact_id", &evidence.value),
                ],
            )?,
        ));
    }
    if receipt.rejection_code.is_empty() {
        events.push((
            AuditAction::FactorAdmitted,
            factor.clone(),
            event_payload(
                "loop.audit.factor_admitted",
                &[
                    ("factor_spec_id", factor),
                    ("decision", "admitted"),
                    ("evidence_artifact_id", &evidence.value),
                ],
            )?,
        ));
    } else {
        events.push((
            AuditAction::FactorRejected,
            factor.clone(),
            event_payload(
                "loop.audit.factor_rejected",
                &[
                    ("factor_spec_id", factor),
                    ("rejection_code", &receipt.rejection_code),
                    ("reason", &command.reason),
                    ("evidence_artifact_id", &evidence.value),
                ],
            )?,
        ));
    }
    for retired in &receipt.states[1..] {
        events.push((
            AuditAction::CommandAccepted,
            retired.factor_spec_id.clone(),
            event_payload(
                "loop.audit.command_accepted",
                &[
                    ("command", "loop.factors.retire"),
                    (
                        "request_id",
                        &context
                            .request_id
                            .as_ref()
                            .ok_or(StoreError::Invalid("factor request"))?
                            .value,
                    ),
                    (
                        "summary",
                        &format!(
                            "replacement by {factor}; prior_revision={}; lifetime_retirements={}",
                            retired.revision - 1,
                            retired.retirements
                        ),
                    ),
                ],
            )?,
        ));
    }
    events.push((AuditAction::CommandAccepted, factor.clone(), event_payload("loop.audit.command_accepted", &[
        ("command", OPERATION), ("request_id", &context.request_id.as_ref().ok_or(StoreError::Invalid("factor request"))?.value),
        ("summary", &format!("context={}; path={}; prior_revision={}; override_requested={}; override_applied={}; retirements={}", command.context_id, if command.expected_revision == 0 { "standard" } else { "readmission" }, command.expected_revision, !command.override_reason.is_empty(), receipt.override_applied, receipt.states.len() - 1)),
    ])?));
    for (action, factor, payload) in events {
        audit::append(
            transaction,
            &store.ledger_id,
            now,
            audit::EventInput {
                actor: principal,
                correlation_id: &context
                    .correlation_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("correlation"))?
                    .value,
                causation_id: &context
                    .causation_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("causation"))?
                    .value,
                action,
                target: AuditTarget {
                    kind: AuditTargetKind::FactorSpecId,
                    value: factor,
                },
                payload,
            },
        )
        .await?;
    }
    Ok(())
}

fn event_payload(
    schema: &str,
    fields: &[(&str, &str)],
) -> StoreResult<loop_core::audit::AuditPayload> {
    use serde::{Serializer, ser::SerializeMap};
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::new(&mut bytes);
    let mut map = serializer
        .serialize_map(Some(fields.len()))
        .map_err(|_| StoreError::Invalid("audit fields"))?;
    for (key, value) in fields {
        map.serialize_entry(key, value)
            .map_err(|_| StoreError::Invalid("audit value"))?;
    }
    map.end().map_err(|_| StoreError::Invalid("audit map"))?;
    Ok(canonicalize_audit_payload(schema, 1, &bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_digest_has_independent_golden() {
        // Independently calculated with Node.js crypto, not this Rust writer.
        let hex: String = active_digest(&[])
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            hex,
            "1519092edd04a3f68553510913c546c1c557c7e3f0e1bff681f1694275b0a25a"
        );
    }

    #[test]
    fn coverage_keeps_large_counts_exact() {
        let mut evidence = AdmissionEvidence {
            eligible_observations: u64::MAX,
            valid_observations: u64::MAX - 1,
            minimum_coverage_bps: 10000,
            semantic_accepted: true,
            ..Default::default()
        };
        assert_eq!(rejection_code(&evidence), "insufficient_coverage");
        evidence.minimum_coverage_bps = 9999;
        assert_eq!(rejection_code(&evidence), "");
    }
}
