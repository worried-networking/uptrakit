use std::time::Duration;

use uptrakit_integration_tests::api_client::ApiClient;
use uptrakit_integration_tests::containers::{
    ControllerContainer, ServiceContainer, test_network_name,
};

/// Verify that an agent enrolls with the controller and appears in the
/// services list.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn agent_enrolls_with_token() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client.register_and_login().await;

    // Start an agent — it will enroll via the bootstrap token.
    let _agent = ServiceContainer::start_agent(&network, controller.container_name()).await;

    // Wait for the agent to appear in the services list.
    let services = client
        .wait_for_service_count(1, Duration::from_secs(60))
        .await;

    assert_eq!(services.len(), 1, "expected exactly 1 service");
    let service = &services[0];
    assert_eq!(service.status, "approved", "agent should be auto-approved");
    assert!(
        service
            .capabilities
            .iter()
            .any(|c| c == "software_discovery"),
        "agent should have software_discovery capability, got: {:?}",
        service.capabilities,
    );
}
