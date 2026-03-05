use std::time::Duration;

use crate::helpers::api_client::ApiClient;
use crate::helpers::containers::{ControllerContainer, ServiceContainer, test_network_name};

/// Verify that the scheduler enrolls as a system service.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn scheduler_enrolls_as_system_service() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client.register_and_login().await;

    // Start the scheduler — it enrolls with the system enrollment token.
    let _scheduler = ServiceContainer::start_scheduler(&network, controller.container_name()).await;

    // The scheduler should appear in the services list.
    let services = client
        .wait_for_service_count(1, Duration::from_secs(60))
        .await;

    assert_eq!(services.len(), 1, "expected exactly 1 service");
    let service = &services[0];
    assert_eq!(
        service.status, "approved",
        "scheduler should be auto-approved"
    );
    assert!(
        service.capabilities.iter().any(|c| c == "scheduler"),
        "scheduler should have scheduler capability, got: {:?}",
        service.capabilities,
    );
}
