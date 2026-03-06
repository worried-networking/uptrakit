use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

/// Request ID extracted from the `x-request-id` header or generated as UUID v7.
///
/// Stored in request/response extensions for downstream middleware and handlers.
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

const REQUEST_ID_HEADER: &str = "x-request-id";

/// Middleware that ensures every request has a unique request ID.
///
/// - Reads the `x-request-id` header from the incoming request; if absent,
///   generates a new UUID v7.
/// - Stores the [`RequestId`] in request extensions (available to handlers).
/// - Creates a `tracing::info_span!` wrapping the request for structured logging.
/// - Sets the `x-request-id` response header so clients can correlate responses.
pub async fn request_id(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

    let method = req.method().clone();
    let path = req.uri().path().to_owned();

    req.extensions_mut().insert(RequestId(id.clone()));

    let span = tracing::info_span!(
        "http.request",
        request_id = %id,
        method = %method,
        path = %path,
    );
    let _guard = span.enter();

    let mut response = next.run(req).await;

    // Propagate request ID to response extensions (for request_log middleware).
    response.extensions_mut().insert(RequestId(id.clone()));

    if let Ok(header_value) = HeaderValue::from_str(&id) {
        response
            .headers_mut()
            .insert(REQUEST_ID_HEADER, header_value);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::middleware as axum_mw;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use http::Request as HttpRequest;
    use tower::ServiceExt;

    async fn echo_request_id(req: Request) -> impl IntoResponse {
        req.extensions()
            .get::<RequestId>()
            .map(|r| r.0.clone())
            .unwrap_or_default()
    }

    fn build_app() -> Router {
        Router::new()
            .route("/test", get(echo_request_id))
            .layer(axum_mw::from_fn(request_id))
    }

    #[tokio::test]
    async fn generates_request_id_when_missing() {
        let app = build_app();
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let header_val = response
            .headers()
            .get("x-request-id")
            .expect("x-request-id header must be set")
            .to_str()
            .unwrap();

        // UUID v7 format: 8-4-4-4-12 hex chars
        assert_eq!(header_val.len(), 36, "must be a valid UUID string");
        assert!(uuid::Uuid::parse_str(header_val).is_ok());
    }

    #[tokio::test]
    async fn preserves_provided_request_id() {
        let app = build_app();
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/test")
                    .header("x-request-id", "my-custom-id-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let header_val = response
            .headers()
            .get("x-request-id")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(header_val, "my-custom-id-123");
    }

    #[tokio::test]
    async fn response_includes_request_id_header() {
        let app = build_app();
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.headers().contains_key("x-request-id"));
    }
}
