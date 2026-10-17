//! Server-pinned manifests backed by verified immutable local files.
#![deny(missing_docs)]

pub(crate) mod data;
pub(crate) mod evaluation;
pub(crate) mod feedback;
pub(crate) mod files;
mod loading;
pub(crate) mod model;
pub(crate) mod numerical;
pub(crate) mod perturbation;
mod policy;
pub(crate) mod portfolio;
pub(crate) mod reconciliation;
pub(crate) mod statistics;
mod verification;

pub use evaluation::{EvaluationPin, EvaluationResolver};
pub use files::{LocalArtifacts, ObjectRef};
pub use policy::TrustedManifests;
pub use portfolio::PortfolioPin;

#[cfg(test)]
pub(crate) mod tests;
