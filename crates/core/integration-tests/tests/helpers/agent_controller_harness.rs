//! Docker-based harness for SPIFFE and cert-rotation integration tests.
//!
//! Wraps [`ControllerContainer`] and [`ServiceContainer`] with higher-level
//! polling helpers that integration tests need but that do not belong in the
//! general-purpose [`ApiClient`].
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "integration test infrastructure: panics are acceptable in harness helpers"
)]

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::api_client::ApiClient;
use super::containers::{ControllerContainer, ServiceContainer, test_network_name};

/// Options for starting an `AgentControllerHarness`.
pub(crate) struct HarnessOptions {
    /// SPIFFE trust domain to configure in the controller.
    ///
    /// Empty string disables SPIFFE SAN validation (controller runs without a
    /// trust domain and `spiffe_id` is never populated in service responses).
    pub trust_domain: String,
}

/// High-level test harness: one controller + N agents on a shared Docker network.
///
/// Dropped: containers are stopped, Docker network is removed.
pub(crate) struct AgentControllerHarness {
    network: String,
    pub controller: ControllerHandle,
}

/// Handle to the running controller — exposes test-utils API operations.
pub(crate) struct ControllerHandle {
    _container: ControllerContainer,
    api: Arc<ApiClient>,
}

/// Handle to a running agent container.
///
/// Created by [`AgentControllerHarness::spawn_agent`].
pub(crate) struct AgentHandle {
    _container: ServiceContainer,
    /// Service IDs that existed before this agent was spawned.
    /// Used by `wait_for_connected` to identify the new service.
    known_before: Vec<Uuid>,
    /// Cached once `wait_for_connected` resolves the new service.
    service_id: std::sync::OnceLock<Uuid>,
    api: Arc<ApiClient>,
}

impl AgentControllerHarness {
    /// Start a controller on a fresh Docker network and return a harness.
    ///
    /// Creates the Docker network, starts the controller, waits for readiness,
    /// and logs in as the first user.
    pub(crate) async fn start_with(opts: HarnessOptions) -> Self {
        let network = test_network_name();
        std::process::Command::new("docker")
            .args(["network", "create", &network])
            .status()
            .expect("create Docker test network");

        let container = if opts.trust_domain.is_empty() {
            ControllerContainer::start(&network).await
        } else {
            ControllerContainer::start_with_trust_domain(&network, &opts.trust_domain).await
        };

        let host_port = container.host_port();
        let registration_token = container.registration_token().map(str::to_owned);

        let mut api = ApiClient::new(host_port);
        api.wait_for_ready(Duration::from_secs(30)).await;
        api.register_and_login_with_token(registration_token.as_deref())
            .await;

        let api = Arc::new(api);
        let controller = ControllerHandle {
            _container: container,
            api,
        };

        Self {
            network,
            controller,
        }
    }

    /// Spawn an agent on the same Docker network.
    ///
    /// Snapshots existing services before starting the container so
    /// `wait_for_connected` can identify the new service unambiguously.
    pub(crate) async fn spawn_agent(&self, _name: &str) -> AgentHandle {
        let known_before: Vec<Uuid> = self
            .controller
            .api
            .list_services()
            .await
            .into_iter()
            .map(|s| s.id)
            .collect();

        let container = ServiceContainer::start_agent(
            &self.network,
            self.controller._container.container_name(),
        )
        .await;

        AgentHandle {
            _container: container,
            known_before,
            service_id: std::sync::OnceLock::new(),
            api: Arc::clone(&self.controller.api),
        }
    }
}

impl Drop for AgentControllerHarness {
    fn drop(&mut self) {
        // Containers are already stopped by their own Drop impls (testcontainers).
        // Remove the Docker network that was created by start_with.
        let _ = std::process::Command::new("docker")
            .args(["network", "rm", &self.network])
            .status();
    }
}

impl ControllerHandle {
    /// Return the SPIFFE identity of a service (calls the detail endpoint).
    pub(crate) async fn identity_of(&self, service_id: Uuid) -> ServiceIdentity {
        let body = self
            .api
            .raw_get(&format!("/api/v1/services/{service_id}"))
            .await;
        ServiceIdentity {
            spiffe_id: body["spiffe_id"].as_str().map(str::to_owned),
        }
    }

    /// Send `RequestCertRenewal` to the connected service via the test-utils endpoint.
    ///
    /// Retries up to 5 times at 500ms intervals if the service is not yet
    /// connected (the message can only be delivered to a live WebSocket).
    /// Panics if the service is not connected after all retries.
    pub(crate) async fn request_cert_renewal(&self, service_id: Uuid) {
        for attempt in 1..=5u32 {
            let status = self
                .api
                .raw_post(&format!(
                    "/api/v1/test/services/{service_id}/request-renewal"
                ))
                .await;
            if status == reqwest::StatusCode::OK {
                return;
            }
            if attempt < 5 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        panic!("request_cert_renewal: service {service_id} not connected after 5 attempts");
    }

    /// Close the WebSocket for a service, triggering its reconnect loop.
    pub(crate) async fn disconnect_service(&self, service_id: Uuid) {
        let status = self
            .api
            .raw_post(&format!("/api/v1/test/services/{service_id}/disconnect"))
            .await;
        assert_eq!(
            status,
            reqwest::StatusCode::OK,
            "disconnect_service returned {status}"
        );
    }

    /// Poll until the service with `service_id` is `Approved` again.
    ///
    /// Used after `disconnect_service` to wait for the agent to reconnect.
    pub(crate) async fn wait_for_service_approved(&self, service_id: Uuid) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let body = self
                .api
                .raw_get(&format!("/api/v1/services/{service_id}"))
                .await;
            if body["status"].as_str() == Some("approved") {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "service {service_id} did not return to Approved within 60s after disconnect"
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

/// SPIFFE identity information for a service.
pub(crate) struct ServiceIdentity {
    pub spiffe_id: Option<String>,
}

impl AgentHandle {
    /// Poll until this agent's service appears as `Approved` in the controller.
    ///
    /// On success, caches the service ID for use by other methods.
    /// Panics if no new `Approved` service appears within 60 seconds.
    pub(crate) async fn wait_for_connected(&self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let current = self.api.list_services().await;
            if let Some(svc) = current
                .iter()
                .find(|s| !self.known_before.contains(&s.id) && s.status.to_string() == "approved")
            {
                self.service_id
                    .set(svc.id)
                    .expect("wait_for_connected called more than once");
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("agent did not appear as Approved within 60s");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Return the service ID cached by `wait_for_connected`.
    ///
    /// Panics if `wait_for_connected` has not been called yet.
    pub(crate) fn service_id(&self) -> Uuid {
        *self
            .service_id
            .get()
            .expect("service_id called before wait_for_connected")
    }

    /// Return the current cert serial from the controller's service detail endpoint.
    pub(crate) async fn cert_serial_number(&self) -> String {
        let id = self.service_id();
        let body = self.api.raw_get(&format!("/api/v1/services/{id}")).await;
        body["cert_serial_number"]
            .as_str()
            .expect("cert_serial_number missing in service detail response")
            .to_owned()
    }

    /// Poll until the cert serial in the controller differs from `before_serial`.
    ///
    /// Panics if the serial does not change within 30 seconds.
    pub(crate) async fn wait_for_cert_renewed(&self, before_serial: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(serial) = self
                .api
                .raw_get(&format!("/api/v1/services/{}", self.service_id()))
                .await["cert_serial_number"]
                .as_str()
            {
                if serial != before_serial {
                    return;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("cert serial did not change from {before_serial} within 30s");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
