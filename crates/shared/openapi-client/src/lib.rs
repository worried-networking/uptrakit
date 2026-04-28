#[macro_use]
mod macros;

#[cfg(feature = "mock")]
pub mod mock;

pub(crate) mod paths;

pub mod access_presets;
pub mod api_tokens;
pub mod audit_logs;
pub mod auth;
pub mod autodiscovery;
pub mod batch_progress_stream;
pub mod device_auth_stream;
pub mod discovery_allowlist;
pub mod enrollment_tokens;
pub mod error;
pub mod events_stream;
pub mod health;
pub mod host_tags;
pub mod hosts;
pub mod notifications;
pub mod oidc_auth;
pub mod oidc_providers;
pub mod permissions;
pub mod pki;
pub mod plugin_configs;
pub mod plugin_type_settings;
pub mod roles;
pub mod scheduler;
pub mod services;
pub mod settings;
pub mod settings_nats;
pub mod settings_provider_github;
pub mod software_items;
pub mod sse;
pub mod surfaces;
pub mod system_alerts;
pub mod system_enrollment_tokens;
pub mod system_services;
pub mod update_batches;
pub mod update_history;
pub mod update_output_stream;
pub mod users;

pub use error::{ClientError, Result};

/// Re-export the shared web API types so that downstream crates (e.g. the CLI)
/// do not need a direct dependency on `uptrakit-web-api-types`.
pub use uptrakit_web_api_types as types;

/// Re-export `DeviceAuthStatus` from `uptrakit-shared-types` for convenience,
/// since it appears in `DeviceAuthPollResponse::status`.
pub use uptrakit_shared_types::DeviceAuthStatus;

/// Re-export `Uuid` so that downstream crates can use the exact same type
/// without adding a direct `uuid` dependency.
pub use uuid::Uuid;

/// Re-export `reqwest::Error` so that downstream crates (e.g. the CLI)
/// do not need a direct dependency on `reqwest`.
pub use reqwest::Error as ReqwestError;

/// Re-export `reqwest::StatusCode` so that downstream crates (e.g. the CLI)
/// do not need a direct dependency on `reqwest` for HTTP status handling.
pub use reqwest::StatusCode;

use rootcause::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

/// Serialize a `StatusCode` as its numeric `u16` value for JSON wire compatibility.
fn serialize_status_code<S: serde::Serializer>(
    status: &reqwest::StatusCode,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_u16(status.as_u16())
}

/// Response from a raw (untyped) API request.
#[derive(Debug, Serialize)]
pub struct RawResponse {
    #[serde(serialize_with = "serialize_status_code")]
    pub status: reqwest::StatusCode,
    pub body: serde_json::Value,
}

/// Configuration for automatic retry on transient failures.
///
/// Apply with [`UptrakitClient::with_retry`]. By default the client fails fast
/// with no retries; call `with_retry(RetryConfig::default())` to enable.
///
/// Retries are applied to:
/// - **HTTP 429 Too Many Requests**: respects the `Retry-After` header if
///   present (numeric seconds only); falls back to `initial_delay`.
/// - **HTTP 5xx Server Error**: exponential backoff starting at `initial_delay`,
///   doubling on each attempt, capped at `max_delay`.
///
/// No retry is attempted for 4xx client errors, network errors, or authentication
/// failures — these are not transient.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Number of additional attempts after the initial request fails.
    /// Default: 3.
    pub max_retries: u32,
    /// Delay before the first retry (and base for exponential backoff).
    /// Default: 1 second.
    pub initial_delay: Duration,
    /// Upper bound on any single inter-retry delay.
    /// Default: 30 seconds.
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Typed HTTP client for the Uptrakit web API.
///
/// Provides compile-time type safety for all API endpoints by using shared
/// request/response types from `uptrakit-web-api-types`.
pub struct UptrakitClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
    retry: Option<RetryConfig>,
}

impl UptrakitClient {
    /// Default connect timeout for the HTTP client (10 seconds).
    const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

    /// Default request timeout for the HTTP client (30 seconds).
    const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

    /// Create a new client. Pass `token: None` for unauthenticated endpoints
    /// (e.g. device auth start/poll).
    ///
    /// `request_timeout` overrides [`DEFAULT_REQUEST_TIMEOUT`] when `Some`.
    ///
    /// [`DEFAULT_REQUEST_TIMEOUT`]: Self::DEFAULT_REQUEST_TIMEOUT
    pub fn new(
        base_url: &str,
        token: Option<&str>,
        insecure: bool,
        request_timeout: Option<Duration>,
    ) -> Result<Self> {
        let timeout = request_timeout.unwrap_or(Self::DEFAULT_REQUEST_TIMEOUT);
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Self::DEFAULT_CONNECT_TIMEOUT)
            .timeout(timeout);
        if insecure {
            builder = builder.tls_danger_accept_invalid_certs(true);
        }
        let http = builder.build().context_to()?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.map(|t| t.to_string()),
            retry: None,
        })
    }

    /// Create a client with a required bearer token.
    pub fn with_token(base_url: &str, token: &str, insecure: bool) -> Result<Self> {
        Self::new(base_url, Some(token), insecure, None)
    }

    /// Enable automatic retry on transient failures (429 and 5xx).
    ///
    /// Returns a new client with the given retry configuration. By default,
    /// the client fails fast with no retries. Retries use exponential backoff
    /// for 5xx errors and respect `Retry-After` headers for 429 errors.
    pub fn with_retry(mut self, config: RetryConfig) -> Self {
        self.retry = Some(config);
        self
    }

    /// Execute a raw (untyped) API request. Used by the CLI `api` escape-hatch command.
    pub async fn raw_request(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<RawResponse> {
        let url = format!("{}{}", self.base_url, path);
        let method = method.to_uppercase();
        let req_method = method
            .parse::<reqwest::Method>()
            .map_err(|e| report!(ClientError::InvalidMethod(e.to_string())))?;

        let mut req = self.http.request(req_method, &url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }

        let resp = req.send().await.context_to()?;
        let status = resp.status();
        let text = resp.text().await.context_to()?;

        let body = if text.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
        };

        Ok(RawResponse { status, body })
    }

    // ── Internal helpers ──────────────────────────────────────────────

    fn token_or_err(&self) -> Result<&str> {
        self.token
            .as_deref()
            .ok_or_else(|| report!(ClientError::NotAuthenticated))
    }

    /// Send a request, retrying automatically on 429 and 5xx responses.
    ///
    /// Without a [`RetryConfig`] (the default), this is a direct single-shot
    /// `send()`. Retries use exponential backoff (5xx) or the `Retry-After`
    /// header (429). 4xx and network errors are never retried.
    async fn send_with_retry(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let Some(retry) = &self.retry else {
            return req.send().await.context_to();
        };

        // Pre-clone the builder for every potential retry before the first send
        // consumes it. `try_clone` returns `None` for streaming bodies; the
        // collected vec will just be shorter, reducing effective retry count.
        let retry_builders: Vec<reqwest::RequestBuilder> = (0..retry.max_retries)
            .map_while(|_| req.try_clone())
            .collect();

        let mut resp = req.send().await.context_to()?;

        for (attempt, retry_req) in retry_builders.into_iter().enumerate() {
            let status = resp.status();
            let delay = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                // Respect Retry-After header; fall back to initial_delay.
                parse_retry_after(&resp)
                    .map(Duration::from_secs)
                    .unwrap_or(retry.initial_delay)
                    .min(retry.max_delay)
            } else if status.is_server_error() {
                // Exponential backoff: initial, 2×initial, 4×initial, …, capped.
                let factor = 1u32.checked_shl(attempt as u32).unwrap_or(u32::MAX);
                retry
                    .initial_delay
                    .saturating_mul(factor)
                    .min(retry.max_delay)
            } else {
                // Not retriable — return as-is.
                return Ok(resp);
            };

            tokio::time::sleep(delay).await;
            resp = retry_req.send().await.context_to()?;
        }

        Ok(resp)
    }

    /// Fetch all pages from a paginated list endpoint, accumulating every item.
    ///
    /// Serialises `base_query` to JSON, then overrides `page` and `per_page`
    /// (set to [`MAX_PER_PAGE`]) on each iteration. Stops when
    /// `page >= total_pages` or the first page reports zero total pages.
    ///
    /// [`MAX_PER_PAGE`]: uptrakit_web_api_types::pagination::MAX_PER_PAGE
    pub(crate) async fn fetch_all_pages<T: DeserializeOwned + Send>(
        &self,
        path: &str,
        base_query: &impl Serialize,
    ) -> Result<Vec<T>> {
        use uptrakit_web_api_types::pagination::{MAX_PER_PAGE, PaginatedResponse};

        let base_value = serde_json::to_value(base_query).context_to()?;
        let mut all: Vec<T> = Vec::new();
        let mut page: u64 = 1;
        loop {
            let mut query = base_value.clone();
            if let Some(obj) = query.as_object_mut() {
                obj.insert("page".to_string(), serde_json::json!(page));
                obj.insert("per_page".to_string(), serde_json::json!(MAX_PER_PAGE));
            }
            let resp: PaginatedResponse<T> = self.get_with_query(path, &query).await?;
            let total_pages = resp.total_pages;
            all.extend(resp.items);
            if page >= total_pages || total_pages == 0 {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.http.get(&url).bearer_auth(self.token_or_err()?);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    async fn get_with_query<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &impl Serialize,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .http
            .get(&url)
            .bearer_auth(self.token_or_err()?)
            .query(query);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    async fn post_json<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .http
            .post(&url)
            .bearer_auth(self.token_or_err()?)
            .json(body);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.http.post(&url).bearer_auth(self.token_or_err()?);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    /// POST without authentication (for device auth endpoints).
    async fn post_json_unauth<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.http.post(&url).json(body);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.http.delete(&url).bearer_auth(self.token_or_err()?);
        let resp = self.send_with_retry(req).await?;
        self.handle_empty_response(resp).await
    }

    #[allow(dead_code)]
    async fn delete_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.http.delete(&url).bearer_auth(self.token_or_err()?);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    async fn delete_with_query(&self, path: &str, query: &impl Serialize) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .http
            .delete(&url)
            .bearer_auth(self.token_or_err()?)
            .query(query);
        let resp = self.send_with_retry(req).await?;
        self.handle_empty_response(resp).await
    }

    #[allow(dead_code)]
    async fn delete_with_query_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &impl Serialize,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .http
            .delete(&url)
            .bearer_auth(self.token_or_err()?)
            .query(query);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    async fn put_json<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .http
            .put(&url)
            .bearer_auth(self.token_or_err()?)
            .json(body);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    /// POST with JSON body, expecting a 204 No Content response.
    async fn post_json_no_content(&self, path: &str, body: &impl Serialize) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let req = self
            .http
            .post(&url)
            .bearer_auth(self.token_or_err()?)
            .json(body);
        let resp = self.send_with_retry(req).await?;
        self.handle_empty_response(resp).await
    }

    /// GET without authentication (for public endpoints).
    async fn get_unauth<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.http.get(&url);
        let resp = self.send_with_retry(req).await?;
        self.handle_response(resp).await
    }

    /// GET without authentication, returning the raw response body as text.
    async fn get_text_unauth(&self, path: &str) -> Result<String> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.http.get(&url);
        let resp = self.send_with_retry(req).await?;
        self.handle_text_response(resp).await
    }

    async fn handle_response<T: DeserializeOwned>(&self, resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = parse_retry_after(&resp);
            bail!(ClientError::RateLimited {
                retry_after_seconds: retry_after,
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!(ClientError::NotAuthenticated);
        }
        let text = resp.text().await.context_to()?;
        if status == reqwest::StatusCode::NOT_FOUND {
            let message = extract_error_message(&text);
            bail!(ClientError::NotFound(message));
        }
        if status.is_client_error() || status.is_server_error() {
            let message = extract_error_message(&text);
            bail!(ClientError::Api { status, message });
        }
        serde_json::from_str(&text).context_to()
    }

    async fn handle_empty_response(&self, resp: reqwest::Response) -> Result<()> {
        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = parse_retry_after(&resp);
            bail!(ClientError::RateLimited {
                retry_after_seconds: retry_after,
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!(ClientError::NotAuthenticated);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            let text = resp.text().await.context_to()?;
            let message = extract_error_message(&text);
            bail!(ClientError::NotFound(message));
        }
        if status.is_client_error() || status.is_server_error() {
            let text = resp.text().await.context_to()?;
            let message = extract_error_message(&text);
            bail!(ClientError::Api { status, message });
        }
        Ok(())
    }

    async fn handle_text_response(&self, resp: reqwest::Response) -> Result<String> {
        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = parse_retry_after(&resp);
            bail!(ClientError::RateLimited {
                retry_after_seconds: retry_after,
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!(ClientError::NotAuthenticated);
        }
        let text = resp.text().await.context_to()?;
        if status == reqwest::StatusCode::NOT_FOUND {
            let message = extract_error_message(&text);
            bail!(ClientError::NotFound(message));
        }
        if status.is_client_error() || status.is_server_error() {
            let message = extract_error_message(&text);
            bail!(ClientError::Api { status, message });
        }
        Ok(text)
    }
}

/// Parse the `Retry-After` header from a response as seconds.
///
/// Only the seconds-delay format (e.g. `Retry-After: 60`) is supported.
/// HTTP-date format and non-numeric values return `None`.
fn parse_retry_after(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
}

/// Extract an error message from a JSON response body, falling back to
/// the raw text when the body is not JSON or has no `error` field.
pub(crate) fn extract_error_message(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v["error"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            if text.is_empty() {
                "Request failed".to_string()
            } else {
                text.to_string()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_error_message_from_json() {
        let text = r#"{"error":"Not found"}"#;
        assert_eq!(extract_error_message(text), "Not found");
    }

    #[test]
    fn extract_error_message_from_json_without_error_field() {
        let text = r#"{"message":"something"}"#;
        assert_eq!(extract_error_message(text), text);
    }

    #[test]
    fn extract_error_message_from_plain_text() {
        let text = "Internal Server Error";
        assert_eq!(extract_error_message(text), "Internal Server Error");
    }

    #[test]
    fn extract_error_message_from_empty() {
        assert_eq!(extract_error_message(""), "Request failed");
    }

    #[test]
    fn base_url_trailing_slash_is_trimmed() {
        let client = UptrakitClient::new("https://example.com/", None, false, None)
            .expect("client creation");
        assert_eq!(client.base_url, "https://example.com");
    }

    #[test]
    fn base_url_without_trailing_slash_is_unchanged() {
        let client =
            UptrakitClient::new("https://example.com", None, false, None).expect("client creation");
        assert_eq!(client.base_url, "https://example.com");
    }

    #[test]
    fn with_token_stores_token() {
        let client = UptrakitClient::with_token("https://example.com", "tok-123", false)
            .expect("client creation");
        assert_eq!(client.token.as_deref(), Some("tok-123"));
    }

    #[test]
    fn new_without_token_stores_none() {
        let client =
            UptrakitClient::new("https://example.com", None, false, None).expect("client creation");
        assert!(client.token.is_none());
    }

    #[test]
    fn token_or_err_returns_token_when_present() {
        let client = UptrakitClient::with_token("https://example.com", "tok", false)
            .expect("client creation");
        assert_eq!(client.token_or_err().expect("token"), "tok");
    }

    #[test]
    fn token_or_err_returns_error_when_absent() {
        let client =
            UptrakitClient::new("https://example.com", None, false, None).expect("client creation");
        let err = client.token_or_err().unwrap_err();
        assert!(
            matches!(err.current_context(), ClientError::NotAuthenticated),
            "expected NotAuthenticated, got: {err}"
        );
    }

    #[test]
    fn parse_retry_after_valid_seconds() {
        let resp = http::Response::builder()
            .status(http::StatusCode::TOO_MANY_REQUESTS)
            .header("Retry-After", "60")
            .body("")
            .unwrap();
        let reqwest_resp = reqwest::Response::from(resp);
        assert_eq!(parse_retry_after(&reqwest_resp), Some(60));
    }

    #[test]
    fn parse_retry_after_missing_header() {
        let resp = http::Response::builder()
            .status(http::StatusCode::TOO_MANY_REQUESTS)
            .body("")
            .unwrap();
        let reqwest_resp = reqwest::Response::from(resp);
        assert_eq!(parse_retry_after(&reqwest_resp), None);
    }

    #[test]
    fn parse_retry_after_non_numeric() {
        let resp = http::Response::builder()
            .status(http::StatusCode::TOO_MANY_REQUESTS)
            .header("Retry-After", "Wed, 21 Oct 2025 07:28:00 GMT")
            .body("")
            .unwrap();
        let reqwest_resp = reqwest::Response::from(resp);
        assert_eq!(parse_retry_after(&reqwest_resp), None);
    }

    #[test]
    fn raw_response_serialization() {
        let resp = RawResponse {
            status: reqwest::StatusCode::OK,
            body: serde_json::json!({"key": "value"}),
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["status"], 200);
        assert_eq!(parsed["body"]["key"], "value");
    }

    #[test]
    fn default_client_has_no_retry() {
        let client = UptrakitClient::new("https://example.com", None, false, None).expect("client");
        assert!(client.retry.is_none());
    }

    #[test]
    fn with_retry_sets_config() {
        let client = UptrakitClient::new("https://example.com", None, false, None)
            .expect("client")
            .with_retry(RetryConfig::default());
        assert!(client.retry.is_some());
    }

    #[test]
    fn retry_config_default_values() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay, Duration::from_secs(1));
        assert_eq!(config.max_delay, Duration::from_secs(30));
    }

    /// Helper: build a client pointing at the given URL with a short retry config.
    #[cfg(test)]
    fn retrying_client(base_url: &str) -> UptrakitClient {
        UptrakitClient::with_token(base_url, "test-token", false)
            .expect("client")
            .with_retry(RetryConfig {
                max_retries: 2,
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
            })
    }

    // ── Retry behaviour tests ──────────────────────────────────────────

    #[tokio::test]
    async fn retry_exhausted_on_repeated_503() {
        use httpmock::prelude::*;
        use uptrakit_web_api_types::pagination::PaginationParams;

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method(GET).path("/api/v1/hosts");
            then.status(503).body(r#"{"error":"down"}"#);
        });

        let params = PaginationParams {
            page: None,
            per_page: None,
        };
        let client = retrying_client(&server.base_url());
        let result = client.list_hosts(&params).await;

        assert!(result.is_err());
        // 1 initial attempt + 2 retries = 3 total calls
        mock.assert_calls(3);
    }

    #[tokio::test]
    async fn no_retry_on_400() {
        use httpmock::prelude::*;
        use uptrakit_web_api_types::pagination::PaginationParams;

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method(GET).path("/api/v1/hosts");
            then.status(400).body(r#"{"error":"bad request"}"#);
        });

        let params = PaginationParams {
            page: None,
            per_page: None,
        };
        let client = retrying_client(&server.base_url());
        let result = client.list_hosts(&params).await;

        assert!(result.is_err());
        mock.assert_calls(1); // no retries for client errors
    }

    #[tokio::test]
    async fn no_retry_on_401() {
        use httpmock::prelude::*;
        use uptrakit_web_api_types::pagination::PaginationParams;

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method(GET).path("/api/v1/hosts");
            then.status(401).body(r#"{"error":"unauthorized"}"#);
        });

        let params = PaginationParams {
            page: None,
            per_page: None,
        };
        let client = retrying_client(&server.base_url());
        let result = client.list_hosts(&params).await;

        assert!(result.is_err());
        mock.assert_calls(1); // no retries for 401
    }

    #[tokio::test]
    async fn retry_exhausted_on_repeated_429() {
        use httpmock::prelude::*;
        use uptrakit_web_api_types::pagination::PaginationParams;

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method(GET).path("/api/v1/hosts");
            then.status(429)
                .header("Retry-After", "1")
                .body(r#"{"error":"rate limited"}"#);
        });

        let params = PaginationParams {
            page: None,
            per_page: None,
        };
        let client = retrying_client(&server.base_url());
        let result = client.list_hosts(&params).await;

        assert!(result.is_err());
        // 1 initial + 2 retries = 3 total calls
        mock.assert_calls(3);
    }

    // ── Pagination tests ──────────────────────────────────────────────

    /// Build a minimal valid `HostResponse`-compatible JSON object.
    fn host_json(id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "machine_id": format!("machine-{id}"),
            "hostname": format!("host-{id}"),
            "friendly_name": format!("Host {id}"),
            "os_type": null,
            "os_version": null,
            "architecture": null,
            "ip_address": null,
            "last_seen_at": null,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "agents": [],
            "tags": []
        })
    }

    fn paginated_hosts_json(
        items: Vec<serde_json::Value>,
        total: u64,
        page: u64,
        total_pages: u64,
    ) -> serde_json::Value {
        serde_json::json!({
            "items": items,
            "total": total,
            "page": page,
            "per_page": 1000,
            "total_pages": total_pages
        })
    }

    #[tokio::test]
    async fn list_all_hosts_multi_page() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;

        let h1 = host_json("550e8400-e29b-41d4-a716-446655440001");
        let h2 = host_json("550e8400-e29b-41d4-a716-446655440002");
        let h3 = host_json("550e8400-e29b-41d4-a716-446655440003");

        server.mock(|when, then| {
            when.method(GET)
                .path("/api/v1/hosts")
                .query_param("page", "1")
                .query_param("per_page", "1000");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(paginated_hosts_json(vec![h1.clone()], 3, 1, 3));
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/api/v1/hosts")
                .query_param("page", "2")
                .query_param("per_page", "1000");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(paginated_hosts_json(vec![h2.clone()], 3, 2, 3));
        });
        server.mock(|when, then| {
            when.method(GET)
                .path("/api/v1/hosts")
                .query_param("page", "3")
                .query_param("per_page", "1000");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(paginated_hosts_json(vec![h3.clone()], 3, 3, 3));
        });

        let client = UptrakitClient::with_token(&server.base_url(), "tok", false).expect("client");
        let all = client.list_all_hosts().await.expect("list_all_hosts");
        assert_eq!(all.len(), 3);
        assert_eq!(
            all[0].machine_id,
            "machine-550e8400-e29b-41d4-a716-446655440001"
        );
        assert_eq!(
            all[2].machine_id,
            "machine-550e8400-e29b-41d4-a716-446655440003"
        );
    }

    #[tokio::test]
    async fn list_all_hosts_single_page() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;
        let h1 = host_json("550e8400-e29b-41d4-a716-000000000001");
        let h2 = host_json("550e8400-e29b-41d4-a716-000000000002");

        server.mock(|when, then| {
            when.method(GET).path("/api/v1/hosts");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(paginated_hosts_json(vec![h1, h2], 2, 1, 1));
        });

        let client = UptrakitClient::with_token(&server.base_url(), "tok", false).expect("client");
        let all = client.list_all_hosts().await.expect("list_all_hosts");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_all_hosts_empty() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;

        server.mock(|when, then| {
            when.method(GET).path("/api/v1/hosts");
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(paginated_hosts_json(vec![], 0, 1, 0));
        });

        let client = UptrakitClient::with_token(&server.base_url(), "tok", false).expect("client");
        let all = client.list_all_hosts().await.expect("list_all_hosts");
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn list_all_hosts_forwards_page_params() {
        use httpmock::prelude::*;
        use uptrakit_web_api_types::pagination::MAX_PER_PAGE;

        let server = MockServer::start_async().await;

        // Verify that page=1 and per_page=MAX_PER_PAGE are sent
        let page_param_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/api/v1/hosts")
                .query_param("page", "1")
                .query_param("per_page", MAX_PER_PAGE.to_string());
            then.status(200)
                .header("Content-Type", "application/json")
                .json_body(paginated_hosts_json(vec![], 0, 1, 0));
        });

        let client = UptrakitClient::with_token(&server.base_url(), "tok", false).expect("client");
        client.list_all_hosts().await.expect("list_all_hosts");

        page_param_mock.assert_calls(1);
    }
}
