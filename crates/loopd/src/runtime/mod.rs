//! Authenticated runtime transport and protected artifact access.
#![deny(missing_docs)]

mod artifacts;
mod authority;
mod capability;
mod deployment;
mod evaluation;
mod portfolio;
mod process;
mod reconciliation;
mod service;
#[cfg(test)]
mod tests;

pub(crate) use artifacts::view_id;
pub use artifacts::{ArtifactBroker, DataPin};
pub use authority::{Identity, JobPin, Role, RuntimeAuthority};
pub use deployment::RuntimeDeployment;
pub use evaluation::FactorExecutor;
pub use portfolio::PortfolioExecutor;
pub(crate) use reconciliation::ValidationTask;
pub use reconciliation::{ReconciliationConfig, ReconciliationExecutor, ValidationPin};
pub use service::{RuntimeService, serve};
