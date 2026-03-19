use std::time::Duration;

use uptrakit_openapi_client::types::services::ServiceStatus;

use crate::helpers::api_client::ApiClient;
use crate::helpers::containers::{ControllerContainer, ServiceContainer, test_network_name};

/// Verify that all four service types enroll concurrently with a single
/// controller and appear in the appropriate services lists.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn all_components_enroll_concurrently() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client
        .register_and_login_with_token(controller.registration_token())
        .await;

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

    // Wait for agent + agent-ssh in the tenant services list (2 services).
    let services = client
        .wait_for_service_count(2, Duration::from_secs(120))
        .await;

    assert_eq!(
        services.len(),
        2,
        "expected 2 tenant services, got {}",
        services.len()
    );

    // Wait for scheduler + mqtt in the system services list (2 external +
    // 1 embedded = 3 total).
    let system_services = client
        .wait_for_system_service_count(3, Duration::from_secs(120))
        .await;

    let external_system: Vec<_> = system_services.iter().filter(|s| !s.is_embedded).collect();
    assert_eq!(
        external_system.len(),
        2,
        "expected 2 external system services, got {}",
        external_system.len()
    );

    // Verify each service type is represented across both lists.
    let tenant_capabilities: Vec<&str> = services
        .iter()
        .flat_map(|s| s.capabilities.iter().map(String::as_str))
        .collect();

    let system_capabilities: Vec<&str> = external_system
        .iter()
        .flat_map(|s| s.capabilities.iter().map(String::as_str))
        .collect();

    assert!(
        tenant_capabilities.contains(&"software_discovery"),
        "missing agent (software_discovery) in tenant capabilities: {tenant_capabilities:?}"
    );
    assert!(
        tenant_capabilities.contains(&"ssh_remote"),
        "missing agent-ssh (ssh_remote) in tenant capabilities: {tenant_capabilities:?}"
    );
    assert!(
        system_capabilities.contains(&"scheduler"),
        "missing scheduler capability in system services: {system_capabilities:?}"
    );
    assert!(
        system_capabilities.contains(&"update_tracking"),
        "missing update_tracking capability in system services: {system_capabilities:?}"
    );

    // All tenant services should be approved.
    for service in &services {
        assert_eq!(
            service.status,
            ServiceStatus::Approved,
            "service {} should be approved, got {:?}",
            service.friendly_name,
            service.status
        );
    }
}
