//! Server-pinned manifests backed by verified immutable local files.
#![deny(missing_docs)]

mod files;
mod loading;
mod model;
mod policy;
mod verification;

pub use files::{LocalArtifacts, ObjectRef};
pub use policy::TrustedManifests;

#[cfg(test)]
mod tests;
