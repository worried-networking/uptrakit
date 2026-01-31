use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use http::StatusCode;
use http::header;

use crate::AppState;

pub async fn ca_cert(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bundle_pem = state.ca_snapshot.borrow().bundle_pem.clone();
    (
        [(header::CONTENT_TYPE, "application/x-pem-file")],
        bundle_pem,
    )
}

pub async fn ca_crl(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let crl_pem = state.crl_pem_cache.read().await.clone();
    if crl_pem.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    ([(header::CONTENT_TYPE, "application/x-pem-file")], crl_pem).into_response()
}
