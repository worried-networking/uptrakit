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
//! use uptrakit_web_api_types::hosts::HostResponse;
//!
//! #[tokio::test]
//! async fn list_hosts_returns_empty() {
//!     let server = MockApiServer::start();
//!     let _m = server.hosts().on_list().ok(&PaginatedResponse::<HostResponse>::default());
//!     let client = server.client();
//!     let result = client.list_hosts(&Default::default()).await.unwrap();
//!     assert_eq!(result.items.len(), 0);
//! }
//! ```

use crate::{StatusCode, UptrakitClient, paths};
use httpmock::{Mock, MockServer};
use serde::Serialize;
use uuid::Uuid;

// ── MockApiServer ──────────────────────────────────────────────────────────

/// Mock HTTP server for testing Uptrakit API client interactions.
///
/// Provides typed section accessors so test code never needs to hard-code API
/// paths. Each section (e.g. `hosts()`, `services()`) returns a thin wrapper
/// with `on_*` methods for every endpoint in that group.
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
        UptrakitClient::new(&self.server.base_url(), None, false, None)
            .expect("mock client creation")
    }

    /// Raw access to the underlying [`MockServer`] for custom scenarios.
    pub fn server(&self) -> &MockServer {
        &self.server
    }

    /// Mock any endpoint by HTTP method and path.
    ///
    /// Use this escape hatch for scenarios not covered by the typed section
    /// helpers. `method` is case-insensitive (e.g. `"GET"`, `"post"`).
    pub fn on(&self, method: &str, path: &str) -> MockEndpoint<'_> {
        MockEndpoint::new(&self.server, method, path)
    }

    // ── Section accessors ──────────────────────────────────────────────────

    /// Authentication endpoints (`/api/v1/auth/…`).
    pub fn auth(&self) -> MockAuth<'_> {
        MockAuth {
            server: &self.server,
        }
    }

    /// API token endpoints (`/api/v1/auth/api-tokens`).
    pub fn api_tokens(&self) -> MockApiTokens<'_> {
        MockApiTokens {
            server: &self.server,
        }
    }

    /// Enrollment token endpoints (`/api/v1/enrollment-tokens`).
    pub fn enrollment_tokens(&self) -> MockEnrollmentTokens<'_> {
        MockEnrollmentTokens {
            server: &self.server,
        }
    }

    /// Health endpoint (`/healthz`).
    pub fn health(&self) -> MockHealth<'_> {
        MockHealth {
            server: &self.server,
        }
    }

    /// Host endpoints (`/api/v1/hosts`).
    pub fn hosts(&self) -> MockHosts<'_> {
        MockHosts {
            server: &self.server,
        }
    }

    /// OIDC authentication endpoints (`/api/v1/auth/oidc/…`).
    pub fn oidc_auth(&self) -> MockOidcAuth<'_> {
        MockOidcAuth {
            server: &self.server,
        }
    }

    /// OIDC provider settings (`/api/v1/settings/oidc-providers`).
    pub fn oidc_providers(&self) -> MockOidcProviders<'_> {
        MockOidcProviders {
            server: &self.server,
        }
    }

    /// PKI endpoints (`/api/v1/pki/…`).
    pub fn pki(&self) -> MockPki<'_> {
        MockPki {
            server: &self.server,
        }
    }

    /// Plugin configuration endpoints (`/api/v1/plugin-configs`).
    pub fn plugin_configs(&self) -> MockPluginConfigs<'_> {
        MockPluginConfigs {
            server: &self.server,
        }
    }

    /// Scheduler endpoints (`/api/v1/scheduler/tasks`).
    pub fn scheduler(&self) -> MockScheduler<'_> {
        MockScheduler {
            server: &self.server,
        }
    }

    /// Service endpoints (`/api/v1/services`).
    pub fn services(&self) -> MockServices<'_> {
        MockServices {
            server: &self.server,
        }
    }

    /// Settings endpoints (`/api/v1/settings`).
    pub fn settings(&self) -> MockSettings<'_> {
        MockSettings {
            server: &self.server,
        }
    }

    /// Software item endpoints (`/api/v1/software-items`).
    pub fn software_items(&self) -> MockSoftwareItems<'_> {
        MockSoftwareItems {
            server: &self.server,
        }
    }

    /// System alert endpoints (`/api/v1/system/alerts`).
    pub fn system_alerts(&self) -> MockSystemAlerts<'_> {
        MockSystemAlerts {
            server: &self.server,
        }
    }

    /// Update history endpoints (`/api/v1/update-history`).
    pub fn update_history(&self) -> MockUpdateHistory<'_> {
        MockUpdateHistory {
            server: &self.server,
        }
    }
}

// ── MockEndpoint ───────────────────────────────────────────────────────────

/// Builder for configuring a mock endpoint response.
///
/// Obtain via the `on_*` methods on a section struct. Finalise by calling one
/// of the response methods (`ok`, `no_content`, `unauthorized`, etc.), which
/// registers the mock and returns an [`httpmock::Mock`] handle for call-count
/// assertions (`mock.assert()`, `mock.assert_hits(n)`).
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
        let Self {
            server,
            method,
            path,
        } = self;
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
        let Self {
            server,
            method,
            path,
        } = self;
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
        let Self {
            server,
            method,
            path,
        } = self;
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

// ── Section: Auth ──────────────────────────────────────────────────────────

/// Mock helpers for authentication endpoints.
pub struct MockAuth<'a> {
    server: &'a MockServer,
}

impl<'a> MockAuth<'a> {
    /// Mock `POST /api/v1/auth/register`.
    pub fn on_register(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::auth::REGISTER)
    }

    /// Mock `POST /api/v1/auth/login`.
    pub fn on_login(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::auth::LOGIN)
    }

    /// Mock `POST /api/v1/auth/refresh`.
    pub fn on_refresh(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::auth::REFRESH)
    }

    /// Mock `POST /api/v1/auth/logout`.
    pub fn on_logout(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::auth::LOGOUT)
    }

    /// Mock `GET /api/v1/auth/me`.
    pub fn on_me(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::auth::ME)
    }

    /// Mock `GET /api/v1/auth/methods`.
    pub fn on_auth_methods(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::auth::METHODS)
    }

    /// Mock `POST /api/v1/auth/device`.
    pub fn on_device_auth_start(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::auth::DEVICE)
    }

    /// Mock `POST /api/v1/auth/device/poll`.
    pub fn on_device_auth_poll(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::auth::DEVICE_POLL)
    }

    /// Mock `POST /api/v1/auth/device/approve`.
    pub fn on_device_auth_approve(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::auth::DEVICE_APPROVE)
    }
}

// ── Section: API Tokens ────────────────────────────────────────────────────

/// Mock helpers for API token endpoints.
pub struct MockApiTokens<'a> {
    server: &'a MockServer,
}

impl<'a> MockApiTokens<'a> {
    /// Mock `GET /api/v1/auth/api-tokens`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::api_tokens::BASE)
    }

    /// Mock `POST /api/v1/auth/api-tokens`.
    pub fn on_create(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::api_tokens::BASE)
    }

    /// Mock `DELETE /api/v1/auth/api-tokens/{id}`.
    pub fn on_revoke(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "DELETE", &paths::api_tokens::by_id(id))
    }
}

// ── Section: Enrollment Tokens ─────────────────────────────────────────────

/// Mock helpers for enrollment token endpoints.
pub struct MockEnrollmentTokens<'a> {
    server: &'a MockServer,
}

impl<'a> MockEnrollmentTokens<'a> {
    /// Mock `GET /api/v1/enrollment-tokens`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::enrollment_tokens::BASE)
    }

    /// Mock `POST /api/v1/enrollment-tokens`.
    pub fn on_create(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::enrollment_tokens::BASE)
    }

    /// Mock `GET /api/v1/enrollment-tokens/{id}`.
    pub fn on_get(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", &paths::enrollment_tokens::by_id(id))
    }

    /// Mock `DELETE /api/v1/enrollment-tokens/{id}`.
    pub fn on_revoke(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "DELETE", &paths::enrollment_tokens::by_id(id))
    }
}

// ── Section: Health ────────────────────────────────────────────────────────

/// Mock helpers for the health endpoint.
pub struct MockHealth<'a> {
    server: &'a MockServer,
}

impl<'a> MockHealth<'a> {
    /// Mock `GET /healthz`.
    pub fn on_healthz(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::health::HEALTHZ)
    }
}

// ── Section: Hosts ─────────────────────────────────────────────────────────

/// Mock helpers for host endpoints.
pub struct MockHosts<'a> {
    server: &'a MockServer,
}

impl<'a> MockHosts<'a> {
    /// Mock `GET /api/v1/hosts`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::hosts::BASE)
    }

    /// Mock `GET /api/v1/hosts/{id}`.
    pub fn on_get(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", &paths::hosts::by_id(id))
    }

    /// Mock `PUT /api/v1/hosts/{id}`.
    pub fn on_update(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", &paths::hosts::by_id(id))
    }

    /// Mock `DELETE /api/v1/hosts/{id}`.
    pub fn on_deactivate(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "DELETE", &paths::hosts::by_id(id))
    }
}

// ── Section: OIDC Auth ─────────────────────────────────────────────────────

/// Mock helpers for OIDC authentication endpoints.
pub struct MockOidcAuth<'a> {
    server: &'a MockServer,
}

impl<'a> MockOidcAuth<'a> {
    /// Mock `GET /api/v1/auth/oidc/{provider_id}/authorize`.
    pub fn on_authorize(&self, provider_id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            self.server,
            "GET",
            &paths::oidc_auth::authorize(provider_id),
        )
    }

    /// Mock `POST /api/v1/auth/oidc/exchange`.
    pub fn on_exchange(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::oidc_auth::EXCHANGE)
    }

    /// Mock `POST /api/v1/auth/oidc/link`.
    pub fn on_link(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::oidc_auth::LINK)
    }

    /// Mock `POST /api/v1/auth/oidc/complete-registration`.
    pub fn on_complete_registration(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::oidc_auth::COMPLETE_REGISTRATION)
    }
}

// ── Section: OIDC Providers ────────────────────────────────────────────────

/// Mock helpers for OIDC provider settings endpoints.
pub struct MockOidcProviders<'a> {
    server: &'a MockServer,
}

impl<'a> MockOidcProviders<'a> {
    /// Mock `GET /api/v1/settings/oidc-providers`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::oidc_providers::BASE)
    }

    /// Mock `POST /api/v1/settings/oidc-providers`.
    pub fn on_create(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::oidc_providers::BASE)
    }

    /// Mock `GET /api/v1/settings/oidc-providers/{id}`.
    pub fn on_get(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", &paths::oidc_providers::by_id(id))
    }

    /// Mock `PUT /api/v1/settings/oidc-providers/{id}`.
    pub fn on_update(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", &paths::oidc_providers::by_id(id))
    }

    /// Mock `DELETE /api/v1/settings/oidc-providers/{id}`.
    pub fn on_delete(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "DELETE", &paths::oidc_providers::by_id(id))
    }

    /// Mock `POST /api/v1/settings/oidc-providers/{id}/activate`.
    pub fn on_activate(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", &paths::oidc_providers::activate(id))
    }

    /// Mock `POST /api/v1/settings/oidc-providers/{id}/deactivate`.
    pub fn on_deactivate(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", &paths::oidc_providers::deactivate(id))
    }
}

// ── Section: PKI ───────────────────────────────────────────────────────────

/// Mock helpers for PKI endpoints.
pub struct MockPki<'a> {
    server: &'a MockServer,
}

impl<'a> MockPki<'a> {
    /// Mock `GET /api/v1/pki/ca.crt`.
    pub fn on_ca_cert(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::pki::CA_CERT)
    }

    /// Mock `GET /api/v1/pki/ca.crl`.
    pub fn on_ca_crl(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::pki::CA_CRL)
    }
}

// ── Section: Plugin Configs ────────────────────────────────────────────────

/// Mock helpers for plugin configuration endpoints.
pub struct MockPluginConfigs<'a> {
    server: &'a MockServer,
}

impl<'a> MockPluginConfigs<'a> {
    /// Mock `GET /api/v1/plugin-configs`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::plugin_configs::BASE)
    }

    /// Mock `POST /api/v1/plugin-configs`.
    pub fn on_create(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::plugin_configs::BASE)
    }

    /// Mock `GET /api/v1/plugin-configs/{id}`.
    pub fn on_get(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", &paths::plugin_configs::by_id(id))
    }

    /// Mock `PUT /api/v1/plugin-configs/{id}`.
    pub fn on_update(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", &paths::plugin_configs::by_id(id))
    }

    /// Mock `DELETE /api/v1/plugin-configs/{id}`.
    pub fn on_delete(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "DELETE", &paths::plugin_configs::by_id(id))
    }
}

// ── Section: Scheduler ─────────────────────────────────────────────────────

/// Mock helpers for scheduler task endpoints.
pub struct MockScheduler<'a> {
    server: &'a MockServer,
}

impl<'a> MockScheduler<'a> {
    /// Mock `GET /api/v1/scheduler/tasks`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::scheduler::BASE)
    }

    /// Mock `GET /api/v1/scheduler/tasks/{id}`.
    pub fn on_get(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", &paths::scheduler::by_id(id))
    }

    /// Mock `PUT /api/v1/scheduler/tasks/{id}`.
    pub fn on_update(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", &paths::scheduler::by_id(id))
    }

    /// Mock `POST /api/v1/scheduler/tasks/{id}/trigger`.
    pub fn on_trigger(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", &paths::scheduler::trigger(id))
    }
}

// ── Section: Services ──────────────────────────────────────────────────────

/// Mock helpers for service endpoints.
pub struct MockServices<'a> {
    server: &'a MockServer,
}

impl<'a> MockServices<'a> {
    /// Mock `GET /api/v1/services`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::services::BASE)
    }

    /// Mock `GET /api/v1/services/{id}`.
    pub fn on_get(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", &paths::services::by_id(id))
    }

    /// Mock `PUT /api/v1/services/{id}`.
    pub fn on_update(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", &paths::services::by_id(id))
    }

    /// Mock `POST /api/v1/services/{id}/approve`.
    pub fn on_approve(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", &paths::services::approve(id))
    }

    /// Mock `POST /api/v1/services/{id}/reject`.
    pub fn on_reject(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", &paths::services::reject(id))
    }

    /// Mock `DELETE /api/v1/services/{id}`.
    pub fn on_remove(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "DELETE", &paths::services::by_id(id))
    }

    /// Mock `POST /api/v1/services/{id}/merge`.
    pub fn on_merge(&self, target_id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", &paths::services::merge(target_id))
    }
}

// ── Section: Settings ──────────────────────────────────────────────────────

/// Mock helpers for settings endpoints.
pub struct MockSettings<'a> {
    server: &'a MockServer,
}

impl<'a> MockSettings<'a> {
    /// Mock `GET /api/v1/settings`.
    pub fn on_get_combined(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::settings::COMBINED)
    }

    /// Mock `GET /api/v1/settings/registration`.
    pub fn on_get_registration(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::settings::REGISTRATION)
    }

    /// Mock `PUT /api/v1/settings/registration`.
    pub fn on_update_registration(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", paths::settings::REGISTRATION)
    }

    /// Mock `GET /api/v1/settings/authentication`.
    pub fn on_get_authentication(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::settings::AUTHENTICATION)
    }

    /// Mock `PUT /api/v1/settings/authentication`.
    pub fn on_update_authentication(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", paths::settings::AUTHENTICATION)
    }

    /// Mock `GET /api/v1/settings/agent-certificates`.
    pub fn on_get_agent_certificates(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::settings::AGENT_CERTIFICATES)
    }

    /// Mock `PUT /api/v1/settings/agent-certificates`.
    pub fn on_update_agent_certificates(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", paths::settings::AGENT_CERTIFICATES)
    }

    /// Mock `GET /api/v1/settings/network`.
    pub fn on_get_network(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::settings::NETWORK)
    }

    /// Mock `PUT /api/v1/settings/network`.
    pub fn on_update_network(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", paths::settings::NETWORK)
    }

    /// Mock `POST /api/v1/settings/rotate-ca`.
    pub fn on_rotate_ca(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::settings::ROTATE_CA)
    }

    /// Mock `POST /api/v1/settings/renew-server-certificate`.
    pub fn on_renew_server_certificate(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::settings::RENEW_SERVER_CERT)
    }
}

// ── Section: Software Items ────────────────────────────────────────────────

/// Mock helpers for software item endpoints.
pub struct MockSoftwareItems<'a> {
    server: &'a MockServer,
}

impl<'a> MockSoftwareItems<'a> {
    /// Mock `GET /api/v1/software-items`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::software_items::BASE)
    }

    /// Mock `POST /api/v1/software-items`.
    pub fn on_create(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::software_items::BASE)
    }

    /// Mock `POST /api/v1/software-items/merge/preview`.
    pub fn on_merge_preview(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::software_items::MERGE_PREVIEW)
    }

    /// Mock `POST /api/v1/software-items/merge/execute`.
    pub fn on_merge_execute(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", paths::software_items::MERGE_EXECUTE)
    }

    /// Mock `GET /api/v1/software-items/{id}`.
    pub fn on_get(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", &paths::software_items::by_id(id))
    }

    /// Mock `PUT /api/v1/software-items/{id}`.
    pub fn on_update(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "PUT", &paths::software_items::by_id(id))
    }

    /// Mock `DELETE /api/v1/software-items/{id}`.
    pub fn on_delete(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "DELETE", &paths::software_items::by_id(id))
    }

    /// Mock `POST /api/v1/software-items/{id}/hosts`.
    pub fn on_assign_hosts(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "POST", &paths::software_items::hosts(id))
    }

    /// Mock `PUT /api/v1/software-items/{item_id}/hosts/{host_id}`.
    pub fn on_update_host_assignment(&self, item_id: &Uuid, host_id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            self.server,
            "PUT",
            &paths::software_items::host(item_id, host_id),
        )
    }

    /// Mock `DELETE /api/v1/software-items/{item_id}/hosts/{host_id}`.
    pub fn on_unassign_host(&self, item_id: &Uuid, host_id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            self.server,
            "DELETE",
            &paths::software_items::host(item_id, host_id),
        )
    }

    /// Mock `POST /api/v1/software-items/{id}/check-versions`.
    pub fn on_check_versions(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            self.server,
            "POST",
            &paths::software_items::check_versions(id),
        )
    }

    /// Mock `POST /api/v1/software-items/{item_id}/hosts/{host_id}/check-versions`.
    pub fn on_check_versions_host(&self, item_id: &Uuid, host_id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            self.server,
            "POST",
            &paths::software_items::host_check_versions(item_id, host_id),
        )
    }

    /// Mock `POST /api/v1/software-items/{item_id}/hosts/{host_id}/update`.
    pub fn on_trigger_update(&self, item_id: &Uuid, host_id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(
            self.server,
            "POST",
            &paths::software_items::host_update(item_id, host_id),
        )
    }
}

// ── Section: System Alerts ─────────────────────────────────────────────────

/// Mock helpers for system alert endpoints.
pub struct MockSystemAlerts<'a> {
    server: &'a MockServer,
}

impl<'a> MockSystemAlerts<'a> {
    /// Mock `GET /api/v1/system/alerts`.
    pub fn on_get(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::system_alerts::ALERTS)
    }
}

// ── Section: Update History ────────────────────────────────────────────────

/// Mock helpers for update history endpoints.
pub struct MockUpdateHistory<'a> {
    server: &'a MockServer,
}

impl<'a> MockUpdateHistory<'a> {
    /// Mock `GET /api/v1/update-history`.
    pub fn on_list(&self) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", paths::update_history::BASE)
    }

    /// Mock `GET /api/v1/update-history/{id}`.
    pub fn on_get(&self, id: &Uuid) -> MockEndpoint<'_> {
        MockEndpoint::new(self.server, "GET", &paths::update_history::by_id(id))
    }
}
