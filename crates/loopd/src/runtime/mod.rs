//! Authenticated runtime transport and protected artifact access.
#![deny(missing_docs)]

mod artifacts;
mod authority;
mod capability;
mod deployment;
mod service;
#[cfg(test)]
mod tests;

pub(crate) use artifacts::view_id;
pub use artifacts::{ArtifactBroker, DataPin};
pub use authority::{Identity, JobPin, Role, RuntimeAuthority};
pub use deployment::RuntimeDeployment;
pub use service::{RuntimeService, serve};
