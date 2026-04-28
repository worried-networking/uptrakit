//! REST API client for verifying controller state in system integration tests.
//!
//! Wraps [`UptrakitClient`](uptrakit_openapi_client::UptrakitClient) from the
//! `uptrakit-openapi-client` crate, adding polling helpers used by system tests.

use std::time::Duration;

use uptrakit_openapi_client::UptrakitClient;
use uptrakit_openapi_client::types::SecretString;
use uptrakit_openapi_client::types::auth::{LoginRequest, RegisterRequest};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::services::{ListServicesQuery, ServiceResponse};
use uptrakit_openapi_client::types::system_services::{
    ListSystemServicesQuery, SystemServiceResponse,
};

/// A thin wrapper around [`UptrakitClient`] with polling helpers for
/// system integration tests.
pub(crate) struct ApiClient {
    base_url: String,
    client: Option<UptrakitClient>,
}

impl ApiClient {
    /// Create a new API client pointing at the controller's mapped host port.
    pub(crate) fn new(controller_port: u16) -> Self {
        let base_url = format!("https://127.0.0.1:{controller_port}");
        Self {
            base_url,
            client: None,
        }
    }

    /// Wait until the controller's health endpoint returns 200.
    ///
    /// Polls `GET /healthz` every 500ms until success or timeout.
    pub(crate) async fn wait_for_ready(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        let unauthenticated =
            UptrakitClient::new(&self.base_url, None, true, Some(Duration::from_secs(5)))
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
        let unauthenticated =
            UptrakitClient::new(&self.base_url, None, true, Some(Duration::from_secs(60)))
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
            UptrakitClient::with_token(&self.base_url, &token, true)
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
}
