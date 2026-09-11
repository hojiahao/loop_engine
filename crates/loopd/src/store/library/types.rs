use loop_protocol::wire::v1::{Actor, ArtifactRef, CommandContext, JobId, PolicyReference};
use prost::Message;
use prost_types::Timestamp;

/// One shared admission/readmission command, never a request to rerun a backtest.
#[derive(Clone, PartialEq, Message)]
pub struct DecideFactor {
    /// Attribution checked against a separately transport-authenticated principal.
    #[prost(message, optional, tag = "1")]
    pub context: Option<CommandContext>,
    /// Registered successful primary IS backtest providing the immutable evidence.
    #[prost(message, optional, tag = "2")]
    pub source_job_id: Option<JobId>,
    /// Explicit immutable research/library context, not `latest`.
    #[prost(string, tag = "3")]
    pub context_id: String,
    /// Zero for first consideration; the prior factor revision for readmission.
    #[prost(uint64, tag = "4")]
    pub expected_revision: u64,
    /// Reason for this decision request; required for ordinary and repeat paths.
    #[prost(string, tag = "5")]
    pub reason: String,
    /// Human justification to waive an actual semantic rejection, otherwise empty.
    #[prost(string, tag = "6")]
    pub override_reason: String,
    /// Independently resolved human approval, required with an override reason.
    #[prost(string, tag = "7")]
    pub override_approval_id: String,
    /// Required absolute deadline, no more than 30 seconds after request time.
    #[prost(message, optional, tag = "8")]
    pub deadline: Option<Timestamp>,
}

/// Verified report materialized by the trusted resolver, not caller metrics.
/// Resolution must prove canonical factor identity, IS-only sample, frozen
/// direction, report/primary-result binding and complete deterministic checks.
#[derive(Clone, PartialEq, Message)]
pub struct AdmissionEvidence {
    /// Immutable report containing coverage, machine filters and semantic review.
    #[prost(message, optional, tag = "1")]
    pub report: Option<ArtifactRef>,
    /// Frozen admission rules under which this report was generated.
    #[prost(message, optional, tag = "2")]
    pub policy: Option<PolicyReference>,
    /// SHA-256 of the registered result manifest, independently verified.
    #[prost(bytes = "vec", tag = "3")]
    pub result_manifest_sha256: Vec<u8>,
    /// Digest of the reviewed sorted active-library IDs and revisions.
    #[prost(bytes = "vec", tag = "4")]
    pub library_sha256: Vec<u8>,
    /// Number of eligible point-in-time security/session observations.
    #[prost(uint64, tag = "5")]
    pub eligible_observations: u64,
    /// Number of finite factor observations among eligible observations.
    #[prost(uint64, tag = "6")]
    pub valid_observations: u64,
    /// Minimum exact coverage in basis points, in 1..=10000.
    #[prost(uint32, tag = "7")]
    pub minimum_coverage_bps: u32,
    /// Empty when all machine gates pass; otherwise a deterministic rejection
    /// code (`deterministic_filter`, `performance`, `correlation`, or `policy`).
    #[prost(string, tag = "8")]
    pub machine_rejection: String,
    /// Completed semantic review decision. Missing/failed review must be an
    /// error from the resolver, not a false or true fabricated decision.
    #[prost(bool, tag = "9")]
    pub semantic_accepted: bool,
    /// Sorted, distinct active factors to retire atomically, at most sixteen.
    #[prost(string, repeated, tag = "10")]
    pub replacements: Vec<String>,
}

/// Persisted per-context projection. Counters describe lifetime events, not PnL.
#[derive(Clone, PartialEq, Message)]
pub struct FactorState {
    /// Canonical FactorSpec ID; includes frozen direction and policy identities.
    #[prost(string, tag = "1")]
    pub factor_spec_id: String,
    /// Monotonic per-factor/context revision.
    #[prost(uint64, tag = "2")]
    pub revision: u64,
    /// `admitted`, `rejected`, or `retired`; stale is a separate provenance state.
    #[prost(string, tag = "3")]
    pub status: String,
    /// Lifetime successful admissions, never reset by rejection or retirement.
    #[prost(uint64, tag = "4")]
    pub admissions: u64,
    /// Lifetime retirements, never reset when readmitted.
    #[prost(uint64, tag = "5")]
    pub retirements: u64,
    /// Original evidence job for this factor, not its replacement's identity.
    #[prost(string, tag = "6")]
    pub source_job_id: String,
}

/// Original committed decision and all resulting projections.
#[derive(Clone, Debug, PartialEq)]
pub struct FactorDecision {
    /// The considered factor followed by sorted replacement retirements.
    pub states: Vec<FactorState>,
    /// Domain rejection code, or empty for admission. Not an infrastructure error.
    pub rejection_code: String,
    /// True only if a human semantic override was actually used.
    pub override_applied: bool,
    /// Original command acceptance time.
    pub accepted_at: Timestamp,
    /// Revalidated historical receipt, not another admission or dispatch.
    pub replayed: bool,
}

/// Trial metadata drawn from a verified current job, including failures.
#[derive(Clone, Debug, PartialEq)]
pub struct FactorTrial {
    /// Durable job identity; retrying submission never adds another trial.
    pub job_id: String,
    /// Registered canonical identity, independent of admission status.
    pub factor_spec_id: String,
    /// Current job state; infrastructure failure is distinct from rejection.
    pub state: i32,
    /// Actual lease executions, zero for work cancelled before execution.
    pub attempt: u32,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct Receipt {
    #[prost(message, optional, tag = "1")]
    pub command: Option<DecideFactor>,
    #[prost(message, optional, tag = "2")]
    pub evidence: Option<AdmissionEvidence>,
    #[prost(message, repeated, tag = "3")]
    pub states: Vec<FactorState>,
    #[prost(string, tag = "4")]
    pub rejection_code: String,
    #[prost(bool, tag = "5")]
    pub override_applied: bool,
    #[prost(message, optional, tag = "6")]
    pub accepted_at: Option<Timestamp>,
    #[prost(message, optional, tag = "7")]
    pub principal: Option<Actor>,
    #[prost(message, repeated, tag = "8")]
    pub previous_states: Vec<FactorState>,
}
