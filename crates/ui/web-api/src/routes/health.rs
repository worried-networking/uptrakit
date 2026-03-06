use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sea_orm::ConnectionTrait;
use serde::Serialize;

use crate::AppState;

#[tracing::instrument(skip_all)]
pub async fn healthz() -> impl IntoResponse {
    "ok"
}

#[derive(Serialize)]
struct ReadinessResponse {
    status: &'static str,
    checks: ReadinessChecks,
}

#[derive(Serialize)]
struct ReadinessChecks {
    database: &'static str,
    ca: &'static str,
}

/// Readiness probe — returns 200 when the service can handle traffic,
/// 503 when a critical dependency is unavailable.
#[tracing::instrument(skip_all)]
pub async fn readyz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_ok = state.db().execute_unprepared("SELECT 1").await.is_ok();

    let ca_ok = !state.ca_snapshot.borrow().bundle_pem.is_empty();

    let response = ReadinessResponse {
        status: if db_ok && ca_ok {
            "ready"
        } else {
            "unavailable"
        },
        checks: ReadinessChecks {
            database: if db_ok { "ok" } else { "unavailable" },
            ca: if ca_ok { "ok" } else { "unavailable" },
        },
    };

    let status = if db_ok && ca_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status, Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use http::Request as HttpRequest;
    use tower::ServiceExt;

    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = Router::new().route("/healthz", get(healthz));
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
