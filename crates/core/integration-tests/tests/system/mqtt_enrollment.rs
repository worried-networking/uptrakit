use std::time::Duration;

use crate::helpers::api_client::ApiClient;
use crate::helpers::containers::{ControllerContainer, ServiceContainer, test_network_name};

/// Verify that the MQTT service enrolls as a system service.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn mqtt_enrolls_as_system_service() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client
        .register_and_login_with_token(controller.registration_token())
        .await;

    // Start the MQTT service — it enrolls with the system enrollment token.
    let _mqtt = ServiceContainer::start_mqtt(&network, controller.container_name()).await;

    // The MQTT service should appear in the services list.
    let services = client
        .wait_for_service_count(1, Duration::from_secs(60))
        .await;

    assert_eq!(services.len(), 1, "expected exactly 1 service");
    let service = &services[0];
    assert_eq!(
        service.status, "approved",
        "mqtt service should be auto-approved"
    );
    assert!(
        service.capabilities.iter().any(|c| c == "update_tracking"),
        "mqtt should have update_tracking capability, got: {:?}",
        service.capabilities,
    );
}
