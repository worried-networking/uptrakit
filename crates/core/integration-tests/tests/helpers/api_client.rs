//! REST API client for verifying controller state in system integration tests.
//!
//! Uses `reqwest` with `danger_accept_invalid_certs` to talk to the
//! controller's self-signed HTTPS endpoint via the mapped host port.

use std::time::Duration;

use serde::Deserialize;

/// A simple REST API client for the controller's HTTP API.
///
/// Used by integration tests to register a user, log in, and query the
/// controller's state (e.g. list enrolled services).
pub struct ApiClient {
    client: reqwest::Client,
    base_url: String,
    access_token: Option<String>,
}

/// Auth response from `POST /api/v1/auth/register` and `POST /api/v1/auth/login`.
#[derive(Deserialize)]
struct AuthResponse {
    access_token: String,
}

/// Paginated response wrapper matching the controller's API format.
#[derive(Deserialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
}

/// Minimal service response — only the fields needed for assertions.
#[derive(Debug, Deserialize)]
pub struct ServiceResponse {
    pub friendly_name: String,
    pub status: String,
    pub capabilities: Vec<String>,
    pub service_label: String,
}

impl ApiClient {
    /// Create a new API client pointing at the controller's mapped host port.
    pub fn new(controller_port: u16) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build reqwest client");

        Self {
            client,
            base_url: format!("https://127.0.0.1:{controller_port}"),
            access_token: None,
        }
    }

    /// Wait until the controller's health endpoint returns 200.
    ///
    /// Polls `GET /healthz` every 500ms until success or timeout.
    pub async fn wait_for_ready(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "controller did not become ready within {}s",
                    timeout.as_secs()
                );
            }

            match self
                .client
                .get(format!("{}/healthz", self.base_url))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => return,
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    }

    /// Register a test user and log in, storing the access token for
    /// subsequent authenticated requests.
    pub async fn register_and_login(&mut self) {
        // Register
        let register_resp = self
            .client
            .post(format!("{}/api/v1/auth/register", self.base_url))
            .json(&serde_json::json!({
                "email": "test@example.com",
                "first_name": "Test",
                "last_name": "User",
                "password": "SecureTestPassword123"
            }))
            .send()
            .await
            .expect("register request");

        assert_eq!(
            register_resp.status().as_u16(),
            201,
            "registration failed: {}",
            register_resp
                .text()
                .await
                .unwrap_or_else(|_| "no body".into())
        );

        // Login
        let login_resp = self
            .client
            .post(format!("{}/api/v1/auth/login", self.base_url))
            .json(&serde_json::json!({
                "email": "test@example.com",
                "password": "SecureTestPassword123"
            }))
            .send()
            .await
            .expect("login request");

        assert_eq!(login_resp.status().as_u16(), 200, "login failed");

        let auth: AuthResponse = login_resp.json().await.expect("parse login response");
        self.access_token = Some(auth.access_token);
    }

    /// List all services visible to the authenticated user.
    ///
    /// Requires a prior call to [`register_and_login`](Self::register_and_login).
    pub async fn list_services(&self) -> Vec<ServiceResponse> {
        let token = self
            .access_token
            .as_ref()
            .expect("must call register_and_login first");

        let resp = self
            .client
            .get(format!("{}/api/v1/services?per_page=100", self.base_url))
            .bearer_auth(token)
            .send()
            .await
            .expect("list services request");

        assert_eq!(resp.status().as_u16(), 200, "list services failed");

        let paginated: PaginatedResponse<ServiceResponse> =
            resp.json().await.expect("parse services response");
        paginated.items
    }

    /// Poll `GET /api/v1/services` until at least `min_count` services appear,
    /// or until `timeout` elapses.
    ///
    /// Returns the service list once the threshold is reached.
    pub async fn wait_for_service_count(
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
}
