use std::time::Duration;

use uptrakit_openapi_client::types::system_services::SystemServiceResponse;

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

    // Wait for the MQTT service to appear alongside both embedded system services.
    let services = client
        .wait_for_system_service_count(3, Duration::from_secs(60))
        .await;

    let external: Vec<&SystemServiceResponse> =
        services.iter().filter(|s| !s.is_embedded).collect();
    assert_eq!(
        external.len(),
        1,
        "expected exactly 1 external system service"
    );

    let mqtt = external[0];
    assert!(
        mqtt.capabilities.iter().any(|c| c == "update_tracking"),
        "mqtt should have update_tracking capability, got: {:?}",
        mqtt.capabilities,
    );
}
