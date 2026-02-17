pub mod api_tokens;
pub mod auth;
pub mod error;
pub mod health;
pub mod hosts;
pub mod oidc_auth;
pub mod oidc_providers;
pub mod pki;
pub mod provider_configs;
pub mod scheduler;
pub mod services;
pub mod settings;
pub mod settings_mqtt;
pub mod software_items;
pub mod system_alerts;
pub mod update_history;

pub use error::{ClientError, Result};

/// Re-export the shared web API types so that downstream crates (e.g. the CLI)
/// do not need a direct dependency on `uptrakit-web-api-types`.
pub use uptrakit_web_api_types as types;

/// Re-export `DeviceAuthStatus` from `uptrakit-shared-types` for convenience,
/// since it appears in `DeviceAuthPollResponse::status`.
pub use uptrakit_shared_types::DeviceAuthStatus;

/// Re-export `ServiceType` from `uptrakit-shared-types` for convenience,
/// since it is used by the enrollment token API.
pub use uptrakit_shared_types::ServiceType;

/// Re-export `Uuid` so that downstream crates can use the exact same type
/// without adding a direct `uuid` dependency.
pub use uuid::Uuid;

use rootcause::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Response from a raw (untyped) API request.
#[derive(Debug, Serialize)]
pub struct RawResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

/// Typed HTTP client for the Uptrakit web API.
///
/// Provides compile-time type safety for all API endpoints by using shared
/// request/response types from `uptrakit-web-api-types`.
pub struct UptrakitClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl UptrakitClient {
    /// Create a new client. Pass `token: None` for unauthenticated endpoints
    /// (e.g. device auth start/poll).
    pub fn new(base_url: &str, token: Option<&str>, insecure: bool) -> Result<Self> {
        let mut builder = reqwest::Client::builder();
        if insecure {
            builder = builder.tls_danger_accept_invalid_certs(true);
        }
        let http = builder.build().context_to()?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.map(|t| t.to_string()),
        })
    }

    /// Create a client with a required bearer token.
    pub fn with_token(base_url: &str, token: &str, insecure: bool) -> Result<Self> {
        Self::new(base_url, Some(token), insecure)
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
        let status = resp.status().as_u16();
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

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(self.token_or_err()?)
            .send()
            .await
            .context_to()?;
        self.handle_response(resp).await
    }

    async fn get_with_query<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &impl Serialize,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(self.token_or_err()?)
            .query(query)
            .send()
            .await
            .context_to()?;
        self.handle_response(resp).await
    }

    async fn post_json<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(self.token_or_err()?)
            .json(body)
            .send()
            .await
            .context_to()?;
        self.handle_response(resp).await
    }

    async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(self.token_or_err()?)
            .send()
            .await
            .context_to()?;
        self.handle_response(resp).await
    }

    /// POST without authentication (for device auth endpoints).
    async fn post_json_unauth<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.post(&url).json(body).send().await.context_to()?;
        self.handle_response(resp).await
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(self.token_or_err()?)
            .send()
            .await
            .context_to()?;
        self.handle_empty_response(resp).await
    }

    async fn delete_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(self.token_or_err()?)
            .send()
            .await
            .context_to()?;
        self.handle_response(resp).await
    }

    async fn delete_with_query(&self, path: &str, query: &impl Serialize) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(self.token_or_err()?)
            .query(query)
            .send()
            .await
            .context_to()?;
        self.handle_empty_response(resp).await
    }

    async fn put_json<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .put(&url)
            .bearer_auth(self.token_or_err()?)
            .json(body)
            .send()
            .await
            .context_to()?;
        self.handle_response(resp).await
    }

    async fn post_empty_with_query<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &impl Serialize,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(self.token_or_err()?)
            .query(query)
            .send()
            .await
            .context_to()?;
        self.handle_response(resp).await
    }

    /// POST with JSON body, expecting a 204 No Content response.
    async fn post_json_no_content(&self, path: &str, body: &impl Serialize) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(self.token_or_err()?)
            .json(body)
            .send()
            .await
            .context_to()?;
        self.handle_empty_response(resp).await
    }

    /// GET without authentication (for public endpoints).
    async fn get_unauth<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.get(&url).send().await.context_to()?;
        self.handle_response(resp).await
    }

    /// GET without authentication, returning the raw response body as text.
    async fn get_text_unauth(&self, path: &str) -> Result<String> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.get(&url).send().await.context_to()?;
        self.handle_text_response(resp).await
    }

    async fn handle_response<T: DeserializeOwned>(&self, resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!(ClientError::RateLimited);
        }
        let text = resp.text().await.context_to()?;
        if status == reqwest::StatusCode::NOT_FOUND {
            let message = extract_error_message(&text);
            bail!(ClientError::NotFound(message));
        }
        if status.is_client_error() || status.is_server_error() {
            let message = extract_error_message(&text);
            bail!(ClientError::Api {
                status: status.as_u16(),
                message,
            });
        }
        serde_json::from_str(&text).context_to()
    }

    async fn handle_empty_response(&self, resp: reqwest::Response) -> Result<()> {
        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!(ClientError::RateLimited);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            let text = resp.text().await.context_to()?;
            let message = extract_error_message(&text);
            bail!(ClientError::NotFound(message));
        }
        if status.is_client_error() || status.is_server_error() {
            let text = resp.text().await.context_to()?;
            let message = extract_error_message(&text);
            bail!(ClientError::Api {
                status: status.as_u16(),
                message,
            });
        }
        Ok(())
    }

    async fn handle_text_response(&self, resp: reqwest::Response) -> Result<String> {
        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!(ClientError::RateLimited);
        }
        let text = resp.text().await.context_to()?;
        if status == reqwest::StatusCode::NOT_FOUND {
            let message = extract_error_message(&text);
            bail!(ClientError::NotFound(message));
        }
        if status.is_client_error() || status.is_server_error() {
            let message = extract_error_message(&text);
            bail!(ClientError::Api {
                status: status.as_u16(),
                message,
            });
        }
        Ok(text)
    }
}

/// Extract an error message from a JSON response body, falling back to
/// the raw text when the body is not JSON or has no `error` field.
fn extract_error_message(text: &str) -> String {
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
        let client =
            UptrakitClient::new("https://example.com/", None, false).expect("client creation");
        assert_eq!(client.base_url, "https://example.com");
    }

    #[test]
    fn base_url_without_trailing_slash_is_unchanged() {
        let client =
            UptrakitClient::new("https://example.com", None, false).expect("client creation");
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
            UptrakitClient::new("https://example.com", None, false).expect("client creation");
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
            UptrakitClient::new("https://example.com", None, false).expect("client creation");
        let err = client.token_or_err().unwrap_err();
        assert!(
            matches!(err.current_context(), ClientError::NotAuthenticated),
            "expected NotAuthenticated, got: {err}"
        );
    }

    #[test]
    fn raw_response_serialization() {
        let resp = RawResponse {
            status: 200,
            body: serde_json::json!({"key": "value"}),
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["status"], 200);
        assert_eq!(parsed["body"]["key"], "value");
    }
}
