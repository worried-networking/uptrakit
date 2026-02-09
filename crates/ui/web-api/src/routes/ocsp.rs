use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use http::header;

use crate::AppState;

const OCSP_RESPONSE_CONTENT_TYPE: &str = "application/ocsp-response";

/// POST /api/v1/pki/ocsp
///
/// Accepts a DER-encoded OCSP request body and returns a DER-encoded OCSP response.
pub async fn ocsp_post(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let snapshot = state.ca_snapshot.borrow().clone();
    let response_der =
        crate::ocsp::build_ocsp_response(&body, &snapshot, &state.ca_key_store, &state.db).await;
    let cache_control = format!("max-age={}, public", crate::ocsp::OCSP_CACHE_MAX_AGE_SECS);

    (
        [
            (header::CONTENT_TYPE, OCSP_RESPONSE_CONTENT_TYPE),
            (header::CACHE_CONTROL, cache_control.as_str()),
        ],
        response_der,
    )
        .into_response()
}

/// GET /api/v1/pki/ocsp/{encoded}
///
/// Accepts a base64url-encoded OCSP request in the URL path and returns a DER-encoded OCSP response.
/// This follows RFC 6960 Section A.1 (HTTP-based OCSP, GET method).
pub async fn ocsp_get(State(state): State<Arc<AppState>>, Path(encoded): Path<String>) -> Response {
    use base64::Engine;

    // RFC 6960 says the request is base64-encoded and then URL-encoded in the path.
    // First URL-decode, then base64-decode.
    let url_decoded = urlencoding::decode(&encoded).unwrap_or_default();
    let request_der = match base64::engine::general_purpose::STANDARD.decode(url_decoded.as_ref()) {
        Ok(der) => der,
        Err(_) => {
            // Return a malformed request OCSP response
            let snapshot = state.ca_snapshot.borrow().clone();
            let err_der =
                crate::ocsp::build_ocsp_response(b"", &snapshot, &state.ca_key_store, &state.db)
                    .await;
            let cache_control = format!("max-age={}, public", crate::ocsp::OCSP_CACHE_MAX_AGE_SECS);
            return (
                [
                    (header::CONTENT_TYPE, OCSP_RESPONSE_CONTENT_TYPE),
                    (header::CACHE_CONTROL, cache_control.as_str()),
                ],
                err_der,
            )
                .into_response();
        }
    };

    let snapshot = state.ca_snapshot.borrow().clone();
    let response_der =
        crate::ocsp::build_ocsp_response(&request_der, &snapshot, &state.ca_key_store, &state.db)
            .await;
    let cache_control = format!("max-age={}, public", crate::ocsp::OCSP_CACHE_MAX_AGE_SECS);

    (
        [
            (header::CONTENT_TYPE, OCSP_RESPONSE_CONTENT_TYPE),
            (header::CACHE_CONTROL, cache_control.as_str()),
        ],
        response_der,
    )
        .into_response()
}
