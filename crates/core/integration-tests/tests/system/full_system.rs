use std::time::Duration;

use uptrakit_integration_tests::api_client::ApiClient;
use uptrakit_integration_tests::containers::{
    ControllerContainer, ServiceContainer, test_network_name,
};

/// Verify that all four service types enroll concurrently with a single
/// controller and appear in the services list.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn all_components_enroll_concurrently() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client.register_and_login().await;

    // Start all four services concurrently.
    let controller_name = controller.container_name().to_string();
    let (agent, scheduler, mqtt, agent_ssh) = tokio::join!(
        ServiceContainer::start_agent(&network, &controller_name),
        ServiceContainer::start_scheduler(&network, &controller_name),
        ServiceContainer::start_mqtt(&network, &controller_name),
        ServiceContainer::start_agent_ssh(&network, &controller_name),
    );

    // Keep containers alive until the test completes.
    let _containers = (agent, scheduler, mqtt, agent_ssh);

    // Wait for all 4 services to appear.
    let services = client
        .wait_for_service_count(4, Duration::from_secs(120))
        .await;

    assert_eq!(
        services.len(),
        4,
        "expected 4 services, got {}",
        services.len()
    );

    // Verify each service type is represented.
    let capabilities: Vec<&str> = services
        .iter()
        .flat_map(|s| s.capabilities.iter().map(String::as_str))
        .collect();

    assert!(
        capabilities.contains(&"software_discovery"),
        "missing agent (software_discovery) in capabilities: {capabilities:?}"
    );
    assert!(
        capabilities.contains(&"scheduler"),
        "missing scheduler capability: {capabilities:?}"
    );
    assert!(
        capabilities.contains(&"mqtt_bridge"),
        "missing mqtt_bridge capability: {capabilities:?}"
    );
    assert!(
        capabilities.contains(&"ssh_remote"),
        "missing agent-ssh (ssh_remote) capability: {capabilities:?}"
    );

    // All should be approved.
    for service in &services {
        assert_eq!(
            service.status, "approved",
            "service {} ({}) should be approved, got {}",
            service.friendly_name, service.service_label, service.status
        );
    }
}
