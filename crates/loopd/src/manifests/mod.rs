//! Server-pinned manifests backed by verified immutable local files.
#![deny(missing_docs)]

pub(crate) mod data;
pub(crate) mod files;
mod loading;
pub(crate) mod model;
mod policy;
mod verification;

pub use files::{LocalArtifacts, ObjectRef};
pub use policy::TrustedManifests;

#[cfg(test)]
mod tests;
