use crate::error::{CliError, Result};
use rootcause::prelude::*;

pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
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

    /// Create a client with a specific bearer token (e.g. JWT from login).
    pub fn with_token(base_url: &str, token: &str, insecure: bool) -> Result<Self> {
        Self::new(base_url, Some(token), insecure)
    }

    /// Make a request. Returns the response body as a serde_json::Value.
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(u16, serde_json::Value)> {
        let url = format!("{}{}", self.base_url, path);

        let method = method.to_uppercase();
        let req_method = method
            .parse::<reqwest::Method>()
            .map_err(|e| report!(CliError::Other(format!("Invalid HTTP method: {e}"))))?;

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

        let value = if text.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
        };

        Ok((status, value))
    }
}
