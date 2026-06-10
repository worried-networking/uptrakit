//! REST API client for verifying controller state in system integration tests.
//!
//! Wraps [`UptrakitClient`](uptrakit_openapi_client::UptrakitClient) from the
//! `uptrakit-openapi-client` crate, adding polling helpers used by system tests.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration test infrastructure: panics are acceptable in API test helpers"
)]
use std::time::Duration;

use uptrakit_openapi_client::UptrakitClient;
use uptrakit_openapi_client::types::SecretString;
use uptrakit_openapi_client::types::audit_logs::{AuditLogListParams, AuditLogResponse};
use uptrakit_openapi_client::types::auth::{LoginRequest, RegisterRequest};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::services::{
    ListServicesQuery, MergeAgentRequest, ServiceResponse,
};
use uptrakit_openapi_client::types::system_services::{
    ListSystemServicesQuery, SystemServiceResponse,
};
use uuid::Uuid;

/// A thin wrapper around [`UptrakitClient`] with polling helpers for
/// system integration tests.
pub(crate) struct ApiClient {
    base_url: String,
    client: Option<UptrakitClient>,
    pki_base_url: Option<String>,
    pub(crate) api_token: Option<String>,
}

impl ApiClient {
    /// Create a new API client pointing at the controller's mapped host port.
    pub(crate) fn new(controller_port: u16) -> Self {
        let base_url = format!("https://127.0.0.1:{controller_port}");
        Self {
            base_url,
            client: None,
            pki_base_url: None,
            api_token: None,
        }
    }

    /// Set the PKI plain-HTTP port. Required before calling `wait_for_pki_generation`.
    pub(crate) fn with_pki_port(mut self, pki_port: u16) -> Self {
        self.pki_base_url = Some(format!("http://127.0.0.1:{pki_port}"));
        self
    }

    /// POST /test/force-reexec — triggers an unconditional reexec.
    ///
    /// The endpoint responds with 202 ACCEPTED before the background task calls exec().
    /// If the response is received, assert it is 2xx to catch mis-wired routes early.
    /// Connection errors after a 2xx response (exec drop) are ignored.
    /// Follow with `wait_for_generation` to confirm the new generation is up.
    pub(crate) async fn force_reexec(&self) {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        match client
            .post(format!("{}/test/force-reexec", self.base_url))
            .send()
            .await
        {
            Ok(resp) => assert!(
                resp.status().is_success(),
                "force_reexec: expected 2xx, got {} — \
                 check UPTRAKIT_TEST_UTILS_ENABLED and route registration",
                resp.status()
            ),
            // Connection reset/EOF after exec() replaces the process image is expected.
            Err(e) => tracing::trace!(error = %e, "force_reexec: connection dropped (expected)"),
        }
    }

    /// Poll GET /healthz every 500ms until X-Reexec-Generation equals `expected`.
    ///
    /// Connection errors are logged at trace level (expected during the reexec gap).
    /// Panics if `timeout` elapses without seeing the expected generation.
    pub(crate) async fn wait_for_generation(&self, expected: u64, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        let url = format!("{}/healthz", self.base_url);

        loop {
            match client.get(&url).send().await {
                Ok(resp) => {
                    let generation: u64 = resp
                        .headers()
                        .get("x-reexec-generation")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    if generation == expected {
                        return;
                    }
                }
                Err(e) => {
                    tracing::trace!(
                        error = %e,
                        "wait_for_generation: connection error (expected during reexec gap)"
                    );
                }
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "controller did not reach generation {} within {}s",
                    expected,
                    timeout.as_secs()
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Poll GET /healthz on the PKI plain-HTTP port every 500ms until
    /// X-Reexec-Generation equals `expected`.
    ///
    /// PKI server is plain HTTP — no TLS required.
    /// Panics if `with_pki_port` was not called, or if `timeout` elapses.
    pub(crate) async fn wait_for_pki_generation(&self, expected: u64, timeout: Duration) {
        let pki_base = self
            .pki_base_url
            .as_deref()
            .expect("pki_base_url not set — call with_pki_port first");
        let url = format!("{pki_base}/healthz");
        let deadline = tokio::time::Instant::now() + timeout;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(5))
            .build()
            .expect("build reqwest client");

        loop {
            match client.get(&url).send().await {
                Ok(resp) => {
                    let generation: u64 = resp
                        .headers()
                        .get("x-reexec-generation")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    if generation == expected {
                        return;
                    }
                }
                Err(e) => {
                    tracing::trace!(
                        error = %e,
                        "wait_for_pki_generation: connection error (expected during reexec gap)"
                    );
                }
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "PKI server did not reach generation {} within {}s",
                    expected,
                    timeout.as_secs()
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Wait until the controller's health endpoint returns 200.
    ///
    /// Polls `GET /healthz` every 500ms until success or timeout.
    pub(crate) async fn wait_for_ready(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        let unauthenticated = UptrakitClient::new(
            &self.base_url,
            None,
            true,
            None,
            Some(Duration::from_secs(5)),
        )
        .expect("build unauthenticated client");

        loop {
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "controller did not become ready within {}s",
                    timeout.as_secs()
                );
            }

            if unauthenticated.healthz().await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Register a test user and log in, optionally supplying a registration token.
    ///
    /// Retries on 500 to handle the SQLite BUSY window that occurs while embedded
    /// services complete their initial DB writes after the controller starts.
    pub(crate) async fn register_and_login_with_token(&mut self, registration_token: Option<&str>) {
        let unauthenticated = UptrakitClient::new(
            &self.base_url,
            None,
            true,
            None,
            Some(Duration::from_secs(60)),
        )
        .expect("build unauthenticated client");

        let register_req = RegisterRequest {
            email: "test@example.com".to_string(),
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            password: SecretString::new("SecureTestPassword123"),
            registration_token: registration_token.map(SecretString::new),
        };

        let auth_resp = {
            let mut last_err = String::new();
            let mut result = None;
            for attempt in 0..10 {
                if attempt > 0 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                match unauthenticated.register(&register_req).await {
                    Ok(resp) => {
                        result = Some(resp);
                        break;
                    }
                    Err(e) => {
                        last_err = format!("{e}");
                        if !last_err.contains("500") {
                            break;
                        }
                    }
                }
            }
            result.unwrap_or_else(|| panic!("registration failed after retries: {last_err}"))
        };

        let token = auth_resp.access_token.expose_secret().to_string();
        self.api_token = Some(token.clone());

        // Verify login works too.
        let login_req = LoginRequest {
            email: "test@example.com".to_string(),
            password: SecretString::new("SecureTestPassword123"),
        };
        let _login_resp = unauthenticated
            .login(&login_req)
            .await
            .expect("login failed");

        // Create an authenticated client using the registration token.
        self.client = Some(
            UptrakitClient::with_token(&self.base_url, &token, true, None)
                .expect("build authenticated client"),
        );
    }

    /// Return a reference to the authenticated client.
    fn authenticated(&self) -> &UptrakitClient {
        self.client
            .as_ref()
            .expect("must call register_and_login_with_token first")
    }

    /// List all tenant services visible to the authenticated user.
    pub(crate) async fn list_services(&self) -> Vec<ServiceResponse> {
        let query = ListServicesQuery {
            capability: None,
            status: None,
            page: None,
            per_page: Some(100),
        };
        let resp: PaginatedResponse<ServiceResponse> = self
            .authenticated()
            .list_services(&query)
            .await
            .expect("list services request");
        resp.items
    }

    /// List all system services visible to the authenticated user.
    pub(crate) async fn list_system_services(&self) -> Vec<SystemServiceResponse> {
        let query = ListSystemServicesQuery {
            capability: None,
            status: None,
            page: None,
            per_page: None,
        };
        let resp: PaginatedResponse<SystemServiceResponse> = self
            .authenticated()
            .list_system_services(&query)
            .await
            .expect("list system services request");
        resp.items
    }

    /// Poll `GET /api/v1/system-services` until at least `min_count` system
    /// services appear, or until `timeout` elapses.
    pub(crate) async fn wait_for_system_service_count(
        &self,
        min_count: usize,
        timeout: Duration,
    ) -> Vec<SystemServiceResponse> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let services = self.list_system_services().await;
            if services.len() >= min_count {
                return services;
            }

            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "expected at least {min_count} system services but found {} after {}s",
                    services.len(),
                    timeout.as_secs()
                );
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Execute a raw GET to the given path and return the response body as JSON.
    ///
    /// Uses the authenticated client. Panics on network error.
    pub(crate) async fn raw_get(&self, path: &str) -> serde_json::Value {
        let resp = self
            .authenticated()
            .raw_request("GET", path, None)
            .await
            .expect("raw GET request");
        resp.body
    }

    /// Execute a raw POST to the given path with no body and return the HTTP status code.
    pub(crate) async fn raw_post(&self, path: &str) -> reqwest::StatusCode {
        let resp = self
            .authenticated()
            .raw_request("POST", path, None)
            .await
            .expect("raw POST request");
        resp.status
    }

    /// Poll `GET /api/v1/services` until at least `min_count` services appear,
    /// or until `timeout` elapses.
    ///
    /// Returns the service list once the threshold is reached.
    pub(crate) async fn wait_for_service_count(
        &self,
        min_count: usize,
        timeout: Duration,
    ) -> Vec<ServiceResponse> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let services = self.list_services().await;
            if services.len() >= min_count {
                return services;
            }

            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "expected at least {min_count} services but found {} after {}s",
                    services.len(),
                    timeout.as_secs()
                );
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// PUT /api/v1/global-settings/oauth — enable OAuth for E2E tests.
    ///
    /// Sets `canonical_host` (triggers auto-enable of `mcp_enabled`) and sets `dcr_enabled = true`
    /// so Dynamic Client Registration works. First GETs current settings to capture the ETag,
    /// then PUTs with `If-Match: <etag>`. The `IfMatch` extractor does strict string comparison.
    ///
    /// Requires a valid API token (call `register_and_login_with_token` first).
    /// Panics if any request fails or the server returns a non-2xx status.
    pub(crate) async fn update_oauth_settings(&self, canonical_host: &str) {
        let token = self
            .api_token
            .as_deref()
            .expect("must call register_and_login_with_token first");
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("build reqwest client");

        let get_resp = client
            .get(format!("{}/api/v1/global-settings/oauth", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("GET /api/v1/global-settings/oauth");
        assert!(
            get_resp.status().is_success(),
            "GET oauth settings: expected 2xx, got {}",
            get_resp.status()
        );
        let etag = get_resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .expect("ETag header on GET oauth settings")
            .to_string();

        let put_resp = client
            .put(format!("{}/api/v1/global-settings/oauth", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .header("If-Match", &etag)
            .json(&serde_json::json!({
                "canonical_host": canonical_host,
                "dcr_enabled": true
            }))
            .send()
            .await
            .expect("PUT /api/v1/global-settings/oauth");
        assert!(
            put_resp.status().is_success(),
            "update_oauth_settings: expected 2xx, got {}",
            put_resp.status()
        );
    }

    /// Merge a pending source service into an approved target service.
    ///
    /// Wraps `POST /api/v1/services/{target_id}/merge`.
    /// Panics if the request fails or the server returns a non-success status.
    pub(crate) async fn merge_service(&self, target_id: Uuid, source_id: Uuid) -> ServiceResponse {
        let req = MergeAgentRequest { source_id };
        self.authenticated()
            .merge_service(&target_id, &req)
            .await
            .expect("merge_service request")
    }

    /// Poll `GET /api/v1/audit-logs` until an entry with the given `action_type`
    /// appears, or until `timeout` elapses.
    ///
    /// Returns the first matching [`AuditLogResponse`] once found.
    /// Panics if no matching entry is found within the timeout.
    pub(crate) async fn wait_for_audit_log_entry(
        &self,
        action_type: &str,
        timeout: Duration,
    ) -> AuditLogResponse {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let mut params = AuditLogListParams::default();
            params.action_type = Some(action_type.to_string());
            params.per_page = Some(10);
            if let Ok(resp) = self.authenticated().list_audit_logs(&params).await
                && let Some(entry) = resp.items.into_iter().next()
            {
                return entry;
            }

            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "no audit log entry with action_type '{}' found within {}s",
                    action_type,
                    timeout.as_secs()
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// POST /oauth/test/auto-approve/{request_id}
    ///
    /// Approves the pending OAuth consent request, returning the authorization code
    /// extracted from the `code=` query parameter of the returned `redirect_uri`.
    ///
    /// Panics if the request fails, returns non-2xx, or `code=` is absent in the URI.
    pub(crate) async fn auto_approve_consent(&self, request_id: &str) -> String {
        let token = self
            .api_token
            .as_deref()
            .expect("must call register_and_login_with_token first");
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("build reqwest client");
        let resp = client
            .post(format!(
                "{}/oauth/test/auto-approve/{request_id}",
                self.base_url
            ))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("POST /oauth/test/auto-approve");
        assert!(
            resp.status().is_success(),
            "auto_approve_consent: expected 2xx, got {}",
            resp.status()
        );
        let body: serde_json::Value = resp.json().await.expect("parse auto-approve response");
        let redirect_uri = body
            .get("redirect_uri")
            .and_then(|v| v.as_str())
            .expect("redirect_uri in response");
        redirect_uri
            .split('?')
            .nth(1)
            .and_then(|q| {
                q.split('&')
                    .find(|p| p.starts_with("code="))
                    .map(|p| p.trim_start_matches("code=").to_owned())
            })
            .expect("code= parameter in redirect_uri")
    }
}
