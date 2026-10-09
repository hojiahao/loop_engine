//! Authenticated runtime transport and protected artifact access.
#![deny(missing_docs)]

mod artifacts;
mod authority;
mod capability;
mod deployment;
mod discovery;
mod evaluation;
pub(crate) mod model_codec;
mod portfolio;
mod process;
mod reconciliation;
mod runs;
mod service;
mod statistics;
#[cfg(test)]
mod tests;

pub(crate) use artifacts::view_id;
pub use artifacts::{ArtifactBroker, DataPin};
pub use authority::{Identity, JobPin, Role, RuntimeAuthority};
pub use deployment::RuntimeDeployment;
pub use discovery::{DiscoveryConfig, DiscoveryExecutor, ProviderConfig};
pub use evaluation::FactorExecutor;
pub use portfolio::PortfolioExecutor;
pub(crate) use reconciliation::ValidationTask;
pub use reconciliation::{ReconciliationConfig, ReconciliationExecutor, ValidationPin};
pub use runs::{RunCatalog, RunConfig};
pub use service::{RuntimeService, serve};
pub(crate) use statistics::StatisticsTask;
pub use statistics::{StatisticsConfig, StatisticsExecutor};
