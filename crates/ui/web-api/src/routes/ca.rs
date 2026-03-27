use axum::extract::State;
use axum::response::IntoResponse;
use http::StatusCode;
use http::header;

use crate::app_state::CertState;
use crate::error_response::error_response;

#[tracing::instrument(skip_all)]
pub async fn ca_cert(State(cert): State<CertState>) -> impl IntoResponse {
    let bundle_pem = cert.ca_snapshot.borrow().bundle_pem.clone();
    (
        [(header::CONTENT_TYPE, "application/x-pem-file")],
        bundle_pem,
    )
}

#[tracing::instrument(skip_all)]
pub async fn ca_crl(State(cert): State<CertState>) -> impl IntoResponse {
    let crl_pem = cert.crl_pem_cache.read().await.clone();
    if crl_pem.is_empty() {
        return error_response(StatusCode::NOT_FOUND, "Not found");
    }
    ([(header::CONTENT_TYPE, "application/x-pem-file")], crl_pem).into_response()
}
