use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use http::header;

use crate::AppState;

pub async fn ca_cert(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bundle_pem = state.ca_snapshot.borrow().bundle_pem.clone();
    (
        [(header::CONTENT_TYPE, "application/x-pem-file")],
        bundle_pem,
    )
}
