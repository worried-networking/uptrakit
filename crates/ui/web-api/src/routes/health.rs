use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sea_orm::ConnectionTrait;
use serde::Serialize;

use crate::app_state::{CertState, DbState};

/// Health check. Returns `200 OK` with body `"ok"` and the `X-Reexec-Generation`
/// response header. The header reflects how many times the controller has re-exec'd
/// in-process since original launch (0 = initial, 1 = first reexec, …). Internal
/// diagnostics only — not part of the public OpenAPI spec.
#[tracing::instrument(skip_all)]
pub async fn healthz() -> impl IntoResponse {
    let generation: u64 = std::env::var("UPTRAKIT_REEXEC_GENERATION")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (
        axum::http::StatusCode::OK,
        [(
            axum::http::HeaderName::from_static("x-reexec-generation"),
            generation.to_string(),
        )],
        "ok",
    )
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
pub async fn readyz(State(db): State<DbState>, State(cert): State<CertState>) -> impl IntoResponse {
    let db_ok = db.db().execute_unprepared("SELECT 1").await.is_ok();

    let ca_ok = !cert.ca_snapshot.borrow().bundle_pem.is_empty();

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
    use http::{Request as HttpRequest, header::HeaderName};
    use tower::ServiceExt;

    #[tokio::test]
    async fn healthz_returns_ok_with_generation_header() {
        // No env var set → handler's unwrap_or(0) yields generation 0.
        // No env mutation needed; no lock required.
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
        let gen_header = response
            .headers()
            .get(HeaderName::from_static("x-reexec-generation"))
            .expect("X-Reexec-Generation header must be present");
        assert_eq!(
            gen_header, "0",
            "generation is 0 when UPTRAKIT_REEXEC_GENERATION is unset"
        );
    }
}
