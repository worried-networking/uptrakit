use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

/// Middleware that sets security headers on every HTTP response.
///
/// Applied as the outermost layer so headers are present on all responses,
/// including error pages and static file serves.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("x-xss-protection", HeaderValue::from_static("0"));
    headers.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'"),
    );
    headers.insert(
        "strict-transport-security",
        HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::middleware as axum_mw;
    use axum::routing::get;
    use http::Request as HttpRequest;
    use tower::ServiceExt;

    fn build_app() -> Router {
        Router::new()
            .route("/test", get(|| async { "hello" }))
            .layer(axum_mw::from_fn(security_headers))
    }

    #[tokio::test]
    async fn sets_all_security_headers() {
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

        let h = response.headers();
        assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(h.get("x-frame-options").unwrap(), "DENY");
        assert_eq!(h.get("x-xss-protection").unwrap(), "0");
        assert_eq!(
            h.get("referrer-policy").unwrap(),
            "strict-origin-when-cross-origin"
        );
        assert_eq!(
            h.get("content-security-policy").unwrap(),
            "default-src 'self'; frame-ancestors 'none'"
        );
        assert_eq!(
            h.get("strict-transport-security").unwrap(),
            "max-age=63072000; includeSubDomains"
        );
        assert_eq!(
            h.get("permissions-policy").unwrap(),
            "camera=(), microphone=(), geolocation=()"
        );
    }
}
