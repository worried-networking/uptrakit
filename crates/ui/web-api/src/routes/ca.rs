use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use http::header;

use crate::AppState;

pub async fn ca_cert(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/x-pem-file")],
        state.ca_pem.clone(),
    )
}
