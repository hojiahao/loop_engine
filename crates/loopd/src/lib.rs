#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as loopd;

pub mod store;

use axum::{Json, Router, routing::get};
use loop_core::ComponentHealth;
use store::SqliteJobStore;

pub fn app() -> Router {
    Router::new().route("/healthz", get(health))
}

/// Readiness-only storage integration. Mutating RPCs remain unregistered until
/// transport authentication and reference authorization are available.
pub fn app_with_store(store: SqliteJobStore) -> Router {
    app().route(
        "/readyz",
        get(move || {
            let store = store.clone();
            async move {
                match store.verify_configuration().await {
                    Ok(()) => axum::http::StatusCode::NO_CONTENT,
                    Err(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE,
                }
            }
        }),
    )
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
