//! Feature-gated mock HTTP server for testing Uptrakit API client interactions.
//!
//! Enable with `features = ["mock"]` in `[dev-dependencies]`. The mock server
//! starts on a random port and is shut down when the [`MockApiServer`] is dropped.
//!
//! # Example
//!
//! ```ignore
//! use uptrakit_openapi_client::mock::MockApiServer;
//! use uptrakit_web_api_types::pagination::PaginatedResponse;
//!
//! #[tokio::test]
//! async fn list_hosts_returns_empty() {
//!     let server = MockApiServer::start();
//!     let _m = server.on_list_hosts().ok(&PaginatedResponse::<()>::default());
//!     let client = server.client();
//!     let result = client.list_hosts(&Default::default()).await.unwrap();
//!     assert_eq!(result.items.len(), 0);
//! }
//! ```

use crate::{StatusCode, UptrakitClient};
use httpmock::{Mock, MockServer};
use serde::Serialize;
use uuid::Uuid;

/// Mock HTTP server for testing Uptrakit API client interactions.
///
/// Provides endpoint-aware convenience methods so test code never needs to
/// know API URL paths. Each `on_*` method returns a [`MockEndpoint`] that
/// can be configured to respond in various ways.
///
/// The server listens on a random port and shuts down on drop.
pub struct MockApiServer {
    server: MockServer,
}

impl MockApiServer {
    /// Start a new mock server on a random port.
    pub fn start() -> Self {
        Self {
            server: MockServer::start(),
        }
    }

    /// Return a client pre-configured with a dummy bearer token and the mock
    /// server URL. TLS verification is disabled since the mock uses plain HTTP.
    pub fn client(&self) -> UptrakitClient {
        UptrakitClient::with_token(&self.server.base_url(), "test-token", false)
            .expect("mock client creation")
    }

    /// Return an unauthenticated client pointing at the mock server.
    pub fn client_unauth(&self) -> UptrakitClient {
        UptrakitClient::new(&self.server.base_url(), None, false)
            .expect("mock client creation")
    }

    /// Raw access to the underlying [`MockServer`] for custom scenarios.
    pub fn server(&self) -> &MockServer {
        &self.server
    }

    // ── Hosts ──────────────────────────────────────────────────────────────

    /// Mock `GET /api/v1/hosts`.
    pub fn on_list_hosts(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "GET", "/api/v1/hosts")
    }

    /// Mock `GET /api/v1/hosts/{id}`.
    pub fn on_get_host(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "GET", &format!("/api/v1/hosts/{id}"))
    }

    // ── Services ───────────────────────────────────────────────────────────

    /// Mock `GET /api/v1/services`.
    pub fn on_list_services(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "GET", "/api/v1/services")
    }

    /// Mock `GET /api/v1/services/{id}`.
    pub fn on_get_service(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "GET", &format!("/api/v1/services/{id}"))
    }

    /// Mock `POST /api/v1/services/{id}/approve`.
    pub fn on_approve_service(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            &self.server,
            "POST",
            &format!("/api/v1/services/{id}/approve"),
        )
    }

    /// Mock `POST /api/v1/services/{id}/reject`.
    pub fn on_reject_service(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            &self.server,
            "POST",
            &format!("/api/v1/services/{id}/reject"),
        )
    }

    /// Mock `DELETE /api/v1/services/{id}`.
    pub fn on_remove_service(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "DELETE", &format!("/api/v1/services/{id}"))
    }

    // ── Software Items ─────────────────────────────────────────────────────

    /// Mock `GET /api/v1/software-items`.
    pub fn on_list_software_items(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "GET", "/api/v1/software-items")
    }

    /// Mock `GET /api/v1/software-items/{id}`.
    pub fn on_get_software_item(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            &self.server,
            "GET",
            &format!("/api/v1/software-items/{id}"),
        )
    }

    // ── Scheduler ──────────────────────────────────────────────────────────

    /// Mock `GET /api/v1/scheduler/tasks`.
    pub fn on_list_scheduled_tasks(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "GET", "/api/v1/scheduler/tasks")
    }

    /// Mock `POST /api/v1/scheduler/tasks/{id}/trigger`.
    pub fn on_trigger_scheduled_task(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            &self.server,
            "POST",
            &format!("/api/v1/scheduler/tasks/{id}/trigger"),
        )
    }

    // ── Health ─────────────────────────────────────────────────────────────

    /// Mock `GET /healthz`.
    pub fn on_healthz(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "GET", "/healthz")
    }

    // ── System Alerts ──────────────────────────────────────────────────────

    /// Mock `GET /api/v1/system/alerts`.
    pub fn on_system_alerts(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, "GET", "/api/v1/system/alerts")
    }

    // ── Generic ────────────────────────────────────────────────────────────

    /// Mock any endpoint by HTTP method and path.
    ///
    /// `method` is case-insensitive (e.g. `"GET"`, `"post"`).
    pub fn on(&self, method: &str, path: &str) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, method, path)
    }
}

/// Builder for configuring a mock endpoint response.
///
/// Obtain via [`MockApiServer::on_*`] methods. Finalise by calling one of the
/// response methods (`ok`, `no_content`, `unauthorized`, etc.), which registers
/// the mock and returns an [`httpmock::Mock`] handle you can use for
/// call-count assertions (`mock.assert()`, `mock.assert_hits(n)`).
pub struct MockEndpoint<'a> {
    server: &'a MockServer,
    method: String,
    path: String,
}

impl<'a> MockEndpoint<'a> {
    fn new(server: &'a MockServer, method: &str, path: &str) -> Self {
        Self {
            server,
            method: method.to_uppercase(),
            path: path.to_string(),
        }
    }

    /// Respond [`StatusCode::OK`] with a JSON-serialised body.
    pub fn ok<T: Serialize>(self, body: &T) -> Mock<'a> {
        let json = serde_json::to_string(body).expect("mock body serialization");
        self.respond_raw(StatusCode::OK, json)
    }

    /// Respond [`StatusCode::NO_CONTENT`] with no body.
    pub fn no_content(self) -> Mock<'a> {
        let Self { server, method, path } = self;
        server.mock(move |when, then| {
            when.method(method.as_str()).path(path.as_str());
            then.status(StatusCode::NO_CONTENT.as_u16());
        })
    }

    /// Respond [`StatusCode::UNAUTHORIZED`].
    pub fn unauthorized(self) -> Mock<'a> {
        self.respond_raw(StatusCode::UNAUTHORIZED, r#"{"error":"Unauthorized"}"#)
    }

    /// Respond [`StatusCode::NOT_FOUND`] with an error message.
    pub fn not_found(self, message: &str) -> Mock<'a> {
        let json = format!(r#"{{"error":{}}}"#, serde_json::to_string(message).unwrap());
        self.respond_raw(StatusCode::NOT_FOUND, json)
    }

    /// Respond [`StatusCode::TOO_MANY_REQUESTS`], optionally including a
    /// `Retry-After` header.
    pub fn rate_limited(self, retry_after: Option<u64>) -> Mock<'a> {
        let Self { server, method, path } = self;
        let retry_after_str = retry_after.map(|s| s.to_string());
        server.mock(move |when, then| {
            when.method(method.as_str()).path(path.as_str());
            let t = then
                .status(StatusCode::TOO_MANY_REQUESTS.as_u16())
                .header("Content-Type", "application/json")
                .body(r#"{"error":"Too many requests"}"#);
            if let Some(ref secs) = retry_after_str {
                t.header("Retry-After", secs.as_str());
            }
        })
    }

    /// Respond [`StatusCode::INTERNAL_SERVER_ERROR`] with an error message.
    pub fn internal_error(self, message: &str) -> Mock<'a> {
        let json = format!(r#"{{"error":{}}}"#, serde_json::to_string(message).unwrap());
        self.respond_raw(StatusCode::INTERNAL_SERVER_ERROR, json)
    }

    /// Respond with the given [`StatusCode`] and a JSON-serialised body.
    pub fn respond<T: Serialize>(self, status: StatusCode, body: &T) -> Mock<'a> {
        let json = serde_json::to_string(body).expect("mock body serialization");
        self.respond_raw(status, json)
    }

    /// Respond with the given [`StatusCode`] and a raw JSON string body.
    pub fn respond_raw(self, status: StatusCode, json: impl Into<String>) -> Mock<'a> {
        let Self { server, method, path } = self;
        let status = status.as_u16();
        let json: String = json.into();
        server.mock(move |when, then| {
            when.method(method.as_str()).path(path.as_str());
            then.status(status)
                .header("Content-Type", "application/json")
                .body(json.as_str());
        })
    }
}
