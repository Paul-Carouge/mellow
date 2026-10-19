//! REST API (axum). Handlers validate input and map domain errors to HTTP; no SQL or business
//! rules live here (ARCHITECTURE.md §4.1).

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use mellow_protocol::{HealthResponse, HealthStatus};
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub(crate) struct AppState {
    pub(crate) pool: SqlitePool,
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .with_state(state)
}

/// Healthy only if the database answers, so a monitor sees a broken disk or key, not just a live process.
async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let database_answers = match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, "health check: the database did not answer");
            false
        }
    };
    let (status_code, status) = if database_answers {
        (StatusCode::OK, HealthStatus::Ok)
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            HealthStatus::DatabaseUnavailable,
        )
    };
    (
        status_code,
        Json(HealthResponse {
            status,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    use super::*;
    use crate::storage::{DatabaseKey, open_encrypted_database};

    async fn health_of(pool: SqlitePool) -> (StatusCode, HealthResponse) {
        let response = router(AppState { pool })
            .oneshot(Request::get("/api/v1/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn health_reports_ok_while_the_database_answers_and_unavailable_once_it_is_closed() {
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(
            &directory.path().join("mellow.db"),
            &DatabaseKey::from_bytes([4; 32]),
        )
        .await
        .unwrap();

        let (status_code, body) = health_of(pool.clone()).await;
        assert_eq!(status_code, StatusCode::OK);
        assert_eq!(body.status, HealthStatus::Ok);
        assert_eq!(body.version, env!("CARGO_PKG_VERSION"));

        pool.close().await;
        let (status_code, body) = health_of(pool).await;
        assert_eq!(status_code, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.status, HealthStatus::DatabaseUnavailable);
    }
}
