use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{ETAG, HeaderValue, IF_MATCH};
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use uptrakit_web_api_types::error::ErrorResponse;

use crate::app_state::AppState;
use crate::extractors::etag_source::EtagSource;
use crate::extractors::if_match::strip_etag;

pub(crate) async fn etag_middleware<S>(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response
where
    S: EtagSource,
{
    let method = req.method().clone();

    // PUT/PATCH: validate If-Match before handing off to handler.
    if method == Method::PUT || method == Method::PATCH {
        let client_etag = match req.headers().get(IF_MATCH) {
            None => {
                return (
                    StatusCode::PRECONDITION_REQUIRED,
                    Json(ErrorResponse {
                        error: "if-match header is required".to_string(),
                        code: Some("if_match.required".to_string()),
                    }),
                )
                    .into_response();
            }
            Some(h) => match h.to_str() {
                Ok(s) => s.to_string(),
                Err(_) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "if-match header contains non-ASCII bytes".to_string(),
                            code: Some("if_match.parse_error".to_string()),
                        }),
                    )
                        .into_response();
                }
            },
        };

        let current = match S::current_etag(&state).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "etag lookup failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "etag lookup failed".to_string(),
                        code: Some("if_match.lookup_failed".to_string()),
                    }),
                )
                    .into_response();
            }
        };

        if strip_etag(&client_etag) != strip_etag(&current) {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "etag mismatch (stale version)".to_string(),
                    code: Some("if_match.stale".to_string()),
                }),
            )
                .into_response();
        }
    }

    let mut response = next.run(req).await;

    // Inject ETag on 2xx responses only.
    if response.status().is_success() {
        let etag_result = if method == Method::GET {
            S::current_etag(&state).await
        } else {
            S::refresh_etag(&state).await
        };
        match etag_result {
            Ok(etag) => {
                if let Ok(value) = HeaderValue::from_str(&etag) {
                    response.headers_mut().insert(ETAG, value);
                } else {
                    tracing::warn!(
                        etag,
                        "etag string is not a valid header value; skipping injection"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "etag refresh failed; response sent without ETag");
            }
        }
    }

    response
}
