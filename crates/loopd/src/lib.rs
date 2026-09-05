use axum::{Json, Router, routing::get};
use loop_core::ComponentHealth;

pub fn app() -> Router {
    Router::new().route("/healthz", get(health))
}

pub async fn health() -> Json<ComponentHealth> {
    Json(ComponentHealth::ready("loopd"))
}

#[cfg(test)]
mod tests {
    use loop_core::HealthStatus;

    use super::*;

    #[tokio::test]
    async fn health_reports_ready() {
        let Json(result) = health().await;
        assert_eq!(result.component, "loopd");
        assert_eq!(result.status, HealthStatus::Ready);
    }
}
