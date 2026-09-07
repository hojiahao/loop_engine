use serde::{Deserialize, Serialize};

pub mod audit;
pub mod factor;
pub mod holdout;

pub const PRODUCT_NAME: &str = "Loop Engine";
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub component: String,
    pub status: HealthStatus,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ready,
    Degraded,
}

impl ComponentHealth {
    pub fn ready(component: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: HealthStatus::Ready,
            version: PRODUCT_VERSION.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_health_is_stable_and_serializable() {
        let health = ComponentHealth::ready("loopd");
        assert_eq!(health.status, HealthStatus::Ready);
        assert_eq!(health.component, "loopd");
        assert!(!health.version.is_empty());
    }
}
