use axum::response::IntoResponse;

#[tracing::instrument(skip_all)]
pub async fn healthz() -> impl IntoResponse {
    "ok"
}
