use std::time::Duration;

use crate::helpers::api_client::ApiClient;
use crate::helpers::containers::{ControllerContainer, test_network_name};

/// Verify that the controller starts and serves the health check endpoint.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn controller_starts_and_serves_health_check() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
}

/// Verify user registration and login flow via the controller API.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored"]
async fn controller_user_registration_and_login() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client
        .register_and_login_with_token(controller.registration_token())
        .await;

    // Verify the token works by listing services (should return empty list).
    let services = client.list_services().await;
    assert!(
        services.is_empty(),
        "fresh controller should have no services, got {}",
        services.len()
    );
}
