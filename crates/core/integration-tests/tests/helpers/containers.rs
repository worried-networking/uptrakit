//! Docker container wrappers for system integration tests.
//!
//! Provides [`ControllerContainer`] and [`ServiceContainer`] that wrap
//! testcontainers to manage the lifecycle of Uptrakit binaries running in
//! Docker containers on a shared network.

use testcontainers::GenericImage;
use testcontainers::ImageExt;
use testcontainers::core::IntoContainerPort;
use testcontainers::core::WaitFor;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::runners::AsyncRunner;

/// Docker image name for the multi-binary test image.
const TEST_IMAGE: &str = "uptrakit-test";

/// Docker image tag.
const TEST_IMAGE_TAG: &str = "latest";

/// Enrollment token for agent and agent-ssh services.
const ENROLLMENT_TOKEN: &str = "test-enrollment-token-do-not-use-in-prod";

/// Enrollment token for system services (scheduler, mqtt).
const SYSTEM_ENROLLMENT_TOKEN: &str = "test-system-token-do-not-use-in-prod";

/// Controller HTTPS port inside the container.
const CONTROLLER_PORT: u16 = 8443;
/// NATS port inside the sidecar container.
const NATS_PORT: u16 = 4222;

/// A running controller container with its mapped host port.
pub(crate) struct ControllerContainer {
    /// Sidecar NATS container required by system services with `nats_access`.
    _nats_container: testcontainers::ContainerAsync<GenericImage>,
    /// The underlying testcontainers handle. Dropping this stops the container.
    _controller_container: testcontainers::ContainerAsync<GenericImage>,
    /// Host port mapped to the controller's HTTPS port.
    host_port: u16,
    /// Container name used for DNS resolution on the Docker network.
    container_name: String,
    /// One-time first-user registration token printed by the controller on startup.
    registration_token: Option<String>,
}

impl ControllerContainer {
    /// Start a controller container on the given Docker network.
    ///
    /// The controller is configured with:
    /// - `--allow-plaintext-secrets` (no master key required)
    /// - `--https-addr [::]:8443`
    /// - A JetStream-enabled NATS sidecar for system services
    /// - Bootstrap enrollment tokens with high max-uses and long TTL
    ///
    /// Waits for the "HTTPS server listening on" log message before returning.
    pub(crate) async fn start(network: &str) -> Self {
        let nats_name = format!("nats-{}", uuid::Uuid::now_v7());
        let nats_container = GenericImage::new("nats", "latest")
            .with_wait_for(WaitFor::Log(
                LogWaitStrategy::stdout_or_stderr("Server is ready").with_times(1),
            ))
            .with_cmd(vec!["-js".to_string()])
            .with_network(network)
            .with_container_name(&nats_name)
            .with_hostname(&nats_name)
            .start()
            .await
            .expect("start nats container");

        let container_name = format!("controller-{}", uuid::Uuid::now_v7());

        // GenericImage methods (with_exposed_port, with_wait_for) must be called
        // before ImageExt methods (with_cmd, with_network, etc.) because
        // ImageExt methods consume GenericImage into ContainerRequest.
        let container = GenericImage::new(TEST_IMAGE, TEST_IMAGE_TAG)
            .with_exposed_port(CONTROLLER_PORT.tcp())
            .with_wait_for(WaitFor::Log(
                LogWaitStrategy::stdout_or_stderr("HTTPS server listening on").with_times(1),
            ))
            .with_cmd(vec![
                "uptrakit-controller".to_string(),
                "--allow-plaintext-secrets".to_string(),
                "--https-addr".to_string(),
                "[::]:8443".to_string(),
                "--nats-url".to_string(),
                format!("nats://{nats_name}:{NATS_PORT}"),
                "--bootstrap-enrollment-token".to_string(),
                ENROLLMENT_TOKEN.to_string(),
                "--bootstrap-enrollment-token-max-uses".to_string(),
                "100".to_string(),
                "--bootstrap-enrollment-token-ttl".to_string(),
                "3600".to_string(),
                "--bootstrap-system-enrollment-token".to_string(),
                SYSTEM_ENROLLMENT_TOKEN.to_string(),
                "--bootstrap-system-enrollment-token-max-uses".to_string(),
                "100".to_string(),
                "--bootstrap-system-enrollment-token-ttl".to_string(),
                "3600".to_string(),
            ])
            .with_network(network)
            .with_container_name(&container_name)
            .with_hostname(&container_name)
            .start()
            .await
            .expect("start controller container");

        let host_port = container
            .get_host_port_ipv4(CONTROLLER_PORT.tcp())
            .await
            .expect("get controller mapped port");

        let registration_token =
            container.stderr_to_vec().await.ok().and_then(|stderr| {
                parse_initial_registration_token(&String::from_utf8_lossy(&stderr))
            });

        Self {
            _nats_container: nats_container,
            _controller_container: container,
            host_port,
            container_name,
            registration_token,
        }
    }

    /// The host port mapped to the controller's HTTPS port.
    pub(crate) fn host_port(&self) -> u16 {
        self.host_port
    }

    /// The container name (used for DNS resolution by other containers).
    pub(crate) fn container_name(&self) -> &str {
        &self.container_name
    }

    /// The initial first-user registration token, if the controller printed one.
    pub(crate) fn registration_token(&self) -> Option<&str> {
        self.registration_token.as_deref()
    }
}

/// A running service container (agent, scheduler, mqtt, or agent-ssh).
pub(crate) struct ServiceContainer {
    /// The underlying testcontainers handle. Dropping this stops the container.
    _container: testcontainers::ContainerAsync<GenericImage>,
}

impl ServiceContainer {
    /// Start an agent container that enrolls with the given controller.
    pub(crate) async fn start_agent(network: &str, controller_name: &str) -> Self {
        Self::start_service(network, "uptrakit-agent", controller_name, ENROLLMENT_TOKEN).await
    }

    /// Start a scheduler container that enrolls as a system service.
    pub(crate) async fn start_scheduler(network: &str, controller_name: &str) -> Self {
        Self::start_service(
            network,
            "uptrakit-scheduler",
            controller_name,
            SYSTEM_ENROLLMENT_TOKEN,
        )
        .await
    }

    /// Start an MQTT container that enrolls as a system service.
    pub(crate) async fn start_mqtt(network: &str, controller_name: &str) -> Self {
        Self::start_service(
            network,
            "uptrakit-mqtt",
            controller_name,
            SYSTEM_ENROLLMENT_TOKEN,
        )
        .await
    }

    /// Start an agent-ssh container.
    ///
    /// The agent-ssh binary requires `--allow-plaintext-secrets` because it
    /// manages its own local secret store.
    pub(crate) async fn start_agent_ssh(network: &str, controller_name: &str) -> Self {
        let container = GenericImage::new(TEST_IMAGE, TEST_IMAGE_TAG)
            .with_wait_for(WaitFor::Log(
                LogWaitStrategy::stdout_or_stderr("enrollment complete, certificate saved to disk")
                    .with_times(1),
            ))
            .with_cmd(vec![
                "uptrakit-agent-ssh".to_string(),
                "--url".to_string(),
                format!("https://{controller_name}:{CONTROLLER_PORT}"),
                "--tofu".to_string(),
                "--allow-plaintext-secrets".to_string(),
            ])
            .with_env_var("UPTRAKIT_ENROLLMENT_TOKEN", ENROLLMENT_TOKEN)
            .with_network(network)
            .start()
            .await
            .expect("start uptrakit-agent-ssh container");

        Self {
            _container: container,
        }
    }

    /// Start a service container with the standard flags.
    ///
    /// All services (agent, scheduler, mqtt) use the same CLI pattern:
    /// `<binary> --url https://<controller>:8443 --tofu`
    /// with `UPTRAKIT_ENROLLMENT_TOKEN` set via env var.
    ///
    /// Waits for the "enrollment complete, certificate saved to disk" log
    /// message from the service SDK.
    async fn start_service(
        network: &str,
        binary: &str,
        controller_name: &str,
        token: &str,
    ) -> Self {
        let container = GenericImage::new(TEST_IMAGE, TEST_IMAGE_TAG)
            .with_wait_for(WaitFor::Log(
                LogWaitStrategy::stdout_or_stderr("enrollment complete, certificate saved to disk")
                    .with_times(1),
            ))
            .with_cmd(vec![
                binary.to_string(),
                "--url".to_string(),
                format!("https://{controller_name}:{CONTROLLER_PORT}"),
                "--tofu".to_string(),
            ])
            .with_env_var("UPTRAKIT_ENROLLMENT_TOKEN", token)
            .with_network(network)
            .start()
            .await
            .unwrap_or_else(|e| panic!("start {binary} container: {e}"));

        Self {
            _container: container,
        }
    }
}

/// Generate a unique Docker network name for a test run.
pub(crate) fn test_network_name() -> String {
    format!("uptrakit-test-{}", uuid::Uuid::now_v7())
}

fn parse_initial_registration_token(stderr: &str) -> Option<String> {
    let mut lines = stderr.lines();
    while let Some(line) = lines.next() {
        if !line.contains("No users found. Use this one-time registration token:") {
            continue;
        }

        for candidate in lines.by_ref() {
            let token = candidate.trim();
            if token.is_empty() || token.chars().all(|ch| ch == '=') {
                continue;
            }
            return Some(token.to_string());
        }
    }

    None
}
