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

    // Verify the token works by listing services — embedded services are
    // always present; no external services should exist on a fresh controller.
    let services = client.list_services().await;
    let external: Vec<_> = services.iter().filter(|s| !s.is_embedded).collect();
    assert!(
        external.is_empty(),
        "fresh controller should have no external services, got {}",
        external.len()
    );
}

/// Verify that the HTTPS and PKI ports remain reachable after two sequential reexecs.
///
/// On cold start the controller runs at generation 0 — note that the WaitFor message
/// `"HTTPS server reusing inherited socket on"` is emitted on every startup (cold or
/// post-reexec) because `lib.rs` always passes the socket as `inherited_listener: Some(_)`,
/// regardless of whether it was actually inherited or freshly bound. Generation only
/// increments when `perform_reexec` actually runs.
///
/// This test exercises:
///   - gen 0 → 1: first reexec via test-utils endpoint; covers the inherited-socket
///                `clear_cloexec` path because the listener originated from `bind()` and
///                must survive the first `exec()`.
///   - gen 1 → 2: second reexec; covers the same `clear_cloexec` path on a socket that
///                was itself inherited via `LISTEN_FDS` (listenfd re-arms `FD_CLOEXEC`
///                in `take_inherited_listeners`, so this path is distinct from gen 0→1).
///
/// Call sites exercised at gen 1→2:
///   - lib.rs clear_cloexec(&https_std) — verified by wait_for_generation(2)
///   - lib.rs clear_cloexec(&pki_std)   — verified by wait_for_pki_generation(2)
///
/// Note: do NOT add start_paused = true — this is a Docker-backed system test
/// that must run on real wall-clock time. start_paused would freeze tokio::time::sleep
/// calls in the helpers and deadlock the test.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests -- --ignored reexec_two_generations_inherit_sockets"]
async fn reexec_two_generations_inherit_sockets() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;
    let client = ApiClient::new(controller.host_port()).with_pki_port(controller.pki_host_port());

    // Baseline: cold-started controller reports generation 0 on both listeners.
    client.wait_for_generation(0, Duration::from_secs(30)).await;
    client
        .wait_for_pki_generation(0, Duration::from_secs(30))
        .await;

    // Gen 0 → 1: first reexec — the listener was freshly bound, so this exercises
    // `clear_cloexec` on a fresh-bind socket (must survive its first exec()).
    client.force_reexec().await;
    client.wait_for_generation(1, Duration::from_secs(60)).await;
    client
        .wait_for_pki_generation(1, Duration::from_secs(60))
        .await;

    // Gen 1 → 2: second reexec — listener was inherited via LISTEN_FDS at gen 1, so
    // listenfd re-armed FD_CLOEXEC on it. This exercises `clear_cloexec` on the
    // inherited-socket path (lib.rs:443 HTTPS, lib.rs:464 PKI). Without the call,
    // the kernel would close the fd on this exec() and the next generation would
    // fall back to fresh-bind (or fail if the port is briefly unavailable).
    client.force_reexec().await;
    client.wait_for_generation(2, Duration::from_secs(60)).await;
    client
        .wait_for_pki_generation(2, Duration::from_secs(60))
        .await;

    // Final sanity: HTTPS still accepts requests at generation 2.
    client.wait_for_ready(Duration::from_secs(5)).await;
}
