use std::time::Duration;

use uptrakit_openapi_client::types::services::ServiceStatus;

use crate::helpers::api_client::ApiClient;
use crate::helpers::containers::{ControllerContainer, ServiceContainer, test_network_name};

/// Verify that an agent enrolls with the controller and appears in the
/// services list.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn agent_enrolls_with_token() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client
        .register_and_login_with_token(controller.registration_token())
        .await;

    // Start an agent — it will enroll via the bootstrap token.
    let _agent = ServiceContainer::start_agent(&network, controller.container_name()).await;

    // Wait for 3 services (2 embedded + 1 external agent).
    let services = client
        .wait_for_service_count(3, Duration::from_secs(60))
        .await;

    let external: Vec<_> = services.iter().filter(|s| !s.is_embedded).collect();
    assert_eq!(external.len(), 1, "expected exactly 1 external service");
    let service = external[0];
    assert_eq!(
        service.status,
        ServiceStatus::Approved,
        "agent should be auto-approved"
    );
    assert!(
        service
            .capabilities
            .iter()
            .any(|c| c == "software_discovery"),
        "agent should have software_discovery capability, got: {:?}",
        service.capabilities,
    );
}
