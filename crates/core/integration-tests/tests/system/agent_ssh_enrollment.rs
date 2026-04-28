use std::time::Duration;

use uptrakit_openapi_client::types::services::ServiceStatus;

use crate::helpers::api_client::ApiClient;
use crate::helpers::containers::{ControllerContainer, ServiceContainer, test_network_name};

/// Verify that agent-ssh enrolls with the controller.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn agent_ssh_enrolls_with_token() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client
        .register_and_login_with_token(controller.registration_token())
        .await;

    // Start agent-ssh — it enrolls with the regular enrollment token.
    let _agent_ssh = ServiceContainer::start_agent_ssh(&network, controller.container_name()).await;

    // Wait for 3 services (2 embedded + 1 external agent-ssh).
    let services = client
        .wait_for_service_count(3, Duration::from_secs(60))
        .await;

    let external: Vec<_> = services.iter().filter(|s| !s.is_embedded).collect();
    assert_eq!(external.len(), 1, "expected exactly 1 external service");
    let service = external[0];
    assert_eq!(
        service.status,
        ServiceStatus::Approved,
        "agent-ssh should be auto-approved"
    );
    assert!(
        service.capabilities.iter().any(|c| c == "ssh_remote"),
        "agent-ssh should have ssh_remote capability, got: {:?}",
        service.capabilities,
    );
}
