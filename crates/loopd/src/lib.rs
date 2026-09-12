#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as loopd;

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;

pub mod manifests;
pub mod research_worker;
pub mod runtime;
pub mod store;

use axum::{Json, Router, routing::get};
use loop_core::ComponentHealth;
use store::PgJobStore;

pub fn app() -> Router {
    Router::new().route("/healthz", get(health))
}

/// Readiness-only HTTP integration. Authenticated job RPCs use the separate,
/// explicitly configured mTLS listener; they are never registered on this router.
pub fn app_with_store(store: PgJobStore) -> Router {
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
