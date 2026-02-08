use axum::http::{HeaderValue, Request, header};

/// Cookie name for the refresh token.
const COOKIE_NAME: &str = "refresh_token";

/// Refresh token cookie max-age in seconds (7 days).
const MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;

/// Cookie path — scoped to auth endpoints only.
const COOKIE_PATH: &str = "/api/v1/auth";

/// Build a `Set-Cookie` header value that stores the refresh token as an
/// `HttpOnly; Secure; SameSite=Strict` cookie.
pub fn set_refresh_token_cookie(token: &str) -> HeaderValue {
    let value = format!(
        "{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Strict; Path={COOKIE_PATH}; Max-Age={MAX_AGE_SECS}"
    );
    // The token is base64url so it is always valid header content.
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Build a `Set-Cookie` header value that clears the refresh token cookie.
pub fn clear_refresh_token_cookie() -> HeaderValue {
    let value =
        format!("{COOKIE_NAME}=; HttpOnly; Secure; SameSite=Strict; Path={COOKIE_PATH}; Max-Age=0");
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Extract the refresh token from the `Cookie` request header.
///
/// Returns `None` if the cookie is absent or has no value.
pub fn extract_refresh_token_from_cookie<B>(req: &Request<B>) -> Option<String> {
    let cookie_header = req.headers().get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("refresh_token=") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;

    #[test]
    fn set_cookie_contains_required_flags() {
        let cookie = set_refresh_token_cookie("test_token_123");
        let val = cookie.to_str().expect("valid header");
        assert!(val.contains("refresh_token=test_token_123"));
        assert!(val.contains("HttpOnly"));
        assert!(val.contains("Secure"));
        assert!(val.contains("SameSite=Strict"));
        assert!(val.contains("Path=/api/v1/auth"));
        assert!(val.contains("Max-Age=604800"));
    }

    #[test]
    fn clear_cookie_sets_max_age_zero() {
        let cookie = clear_refresh_token_cookie();
        let val = cookie.to_str().expect("valid header");
        assert!(val.contains("refresh_token=;"));
        assert!(val.contains("Max-Age=0"));
    }

    #[test]
    fn extract_from_cookie_header() {
        let req = HttpRequest::builder()
            .header("Cookie", "other=abc; refresh_token=my_token; session=xyz")
            .body(())
            .expect("valid request");
        assert_eq!(
            extract_refresh_token_from_cookie(&req),
            Some("my_token".to_string())
        );
    }

    #[test]
    fn extract_returns_none_when_absent() {
        let req = HttpRequest::builder()
            .header("Cookie", "other=abc; session=xyz")
            .body(())
            .expect("valid request");
        assert_eq!(extract_refresh_token_from_cookie(&req), None);
    }

    #[test]
    fn extract_returns_none_without_cookie_header() {
        let req = HttpRequest::builder()
            .body(())
            .expect("valid request");
        assert_eq!(extract_refresh_token_from_cookie(&req), None);
    }
}
