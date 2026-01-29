use axum::response::IntoResponse;

pub async fn healthz() -> impl IntoResponse {
    "ok"
}
