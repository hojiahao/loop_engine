//! Immutable research fingerprints and fail-closed metric freshness checks.
//!
//! Frozen and current inputs must be resolved by the owning service, not copied
//! from caller metadata. Equality is not authorization or proof of execution.
#![deny(missing_docs)]

use std::fmt;

use crate::wire::v1::ResearchProvenanceFingerprint;

/// One independently identity-bearing input, in stable protocol field order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProvenanceComponent {
    /// Executable research source manifest.
    SourceCode,
    /// Versioned numerical operator definitions.
    OperatorRegistry,
    /// Resolved research configuration and policies.
    Configuration,
    /// Immutable input-data manifest.
    DataManifest,
    /// Exact exchange-session calendar.
    TradingCalendar,
    /// Resolved execution environment manifest.
    Environment,
}

impl ProvenanceComponent {
    /// Components in the order declared by `ResearchProvenanceFingerprint`.
    pub const ALL: [Self; 6] = [
        Self::SourceCode,
        Self::OperatorRegistry,
        Self::Configuration,
        Self::DataManifest,
        Self::TradingCalendar,
        Self::Environment,
    ];

    /// Stable, language-neutral field name, without the digest suffix.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourceCode => "source_code",
            Self::OperatorRegistry => "operator_registry",
            Self::Configuration => "configuration",
            Self::DataManifest => "data_manifest",
            Self::TradingCalendar => "trading_calendar",
            Self::Environment => "environment",
        }
    }
}

/// Validated owned bytes; later mutation of the wire DTO cannot change this value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvenanceSnapshot {
    digests: [[u8; 32]; 6],
}

impl TryFrom<&ResearchProvenanceFingerprint> for ProvenanceSnapshot {
    type Error = ProvenanceError;

    fn try_from(value: &ResearchProvenanceFingerprint) -> Result<Self, Self::Error> {
        let fields = [
            value.source_code_sha256.as_ref(),
            value.operator_registry_sha256.as_ref(),
            value.configuration_sha256.as_ref(),
            value.data_manifest_sha256.as_ref(),
            value.trading_calendar_sha256.as_ref(),
            value.environment_sha256.as_ref(),
        ];
        let mut digests = [[0; 32]; 6];
        for (index, (component, field)) in
            ProvenanceComponent::ALL.into_iter().zip(fields).enumerate()
        {
            digests[index] = field
                .and_then(|digest| digest.value.as_slice().try_into().ok())
                .ok_or(ProvenanceError::InvalidDigest(component))?;
        }
        Ok(Self { digests })
    }
}

impl ProvenanceSnapshot {
    /// Return every changed component in stable field order; never truncate.
    pub fn differences(&self, other: &Self) -> Vec<ProvenanceComponent> {
        ProvenanceComponent::ALL
            .into_iter()
            .enumerate()
            .filter_map(|(index, component)| {
                (self.digests[index] != other.digests[index]).then_some(component)
            })
            .collect()
    }
}

/// Metadata freshness only; this value never grants access or factor admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProvenanceAssessment {
    /// All six fields match a service-resolved current context.
    Current,
    /// Valid historical evidence does not match the requested current context.
    Stale(Vec<ProvenanceComponent>),
    /// No authoritative current context was available.
    Unresolved,
}

impl ProvenanceAssessment {
    /// Refuse stale or unresolved metrics at a current-result consumption boundary.
    ///
    /// # Errors
    /// Returns `Stale` with every changed field, or `UnresolvedCurrent`.
    pub fn require_current(&self) -> Result<(), ProvenanceError> {
        match self {
            Self::Current => Ok(()),
            Self::Stale(changed) => Err(ProvenanceError::Stale(changed.clone())),
            Self::Unresolved => Err(ProvenanceError::UnresolvedCurrent),
        }
    }
}

/// Integrity and freshness failures, never deterministic factor rejection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProvenanceError {
    /// A required digest is absent or does not contain exactly 32 bytes.
    InvalidDigest(ProvenanceComponent),
    /// A result does not describe the immutable inputs of its original run.
    RecordingMismatch(Vec<ProvenanceComponent>),
    /// Historical metrics cannot be used as current results.
    Stale(Vec<ProvenanceComponent>),
    /// The owner could not resolve the current computational inputs.
    UnresolvedCurrent,
}

impl fmt::Display for ProvenanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDigest(component) => {
                write!(
                    formatter,
                    "invalid provenance digest: {}",
                    component.as_str()
                )
            }
            Self::RecordingMismatch(changed) => {
                write!(
                    formatter,
                    "recorded provenance differs from frozen inputs: {changed:?}"
                )
            }
            Self::Stale(changed) => write!(formatter, "research metrics are stale: {changed:?}"),
            Self::UnresolvedCurrent => {
                formatter.write_str("current research provenance is unresolved")
            }
        }
    }
}

impl std::error::Error for ProvenanceError {}

/// Check original-run integrity first, then freshness against resolved inputs.
///
/// The same factor, backtest specification, sample and seed must be resolved by
/// the caller separately. Missing current context never implies freshness.
/// This pure operation performs no I/O and does not rewrite historical evidence.
///
/// # Errors
/// Returns `RecordingMismatch` even when the current context is unavailable or
/// happens to match the incorrectly recorded result.
pub fn assess_provenance(
    recorded: &ProvenanceSnapshot,
    frozen: &ProvenanceSnapshot,
    current: Option<&ProvenanceSnapshot>,
) -> Result<ProvenanceAssessment, ProvenanceError> {
    let mismatch = recorded.differences(frozen);
    if !mismatch.is_empty() {
        return Err(ProvenanceError::RecordingMismatch(mismatch));
    }
    let Some(current) = current else {
        return Ok(ProvenanceAssessment::Unresolved);
    };
    let changed = recorded.differences(current);
    if changed.is_empty() {
        Ok(ProvenanceAssessment::Current)
    } else {
        Ok(ProvenanceAssessment::Stale(changed))
    }
}
