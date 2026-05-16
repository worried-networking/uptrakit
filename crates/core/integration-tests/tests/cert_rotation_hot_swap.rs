//! End-to-end verification that Agent cert renewal via the resolver hot-swap
//! path keeps the existing WebSocket session alive, then presents the new cert
//! on the next TLS handshake.
//!
//! Run:
//! ```sh
//! docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
//! cargo test -p uptrakit-integration-tests cert_rotation_hot_swap -- --ignored --nocapture
//! ```
// Shared helpers include members not used by this test (e.g. start_scheduler).
#![expect(
    dead_code,
    reason = "shared test helpers include members not exercised by this test binary"
)]

mod helpers;

use crate::helpers::agent_controller_harness::{AgentControllerHarness, HarnessOptions};

/// Verify that cert renewal via the resolver hot-swap path keeps the session
/// alive and the new cert is presented on the next TLS handshake.
///
/// Sequence:
/// 1. Spawn agent, wait for it to be Approved.
/// 2. Record the current cert serial (before renewal).
/// 3. Trigger renewal via test-utils endpoint.
/// 4. Wait for the cert serial to change in the controller DB (hot-swap complete).
/// 5. Verify the service is still Approved (no session disruption during renewal).
/// 6. Force a disconnect to trigger a new TLS handshake.
/// 7. Wait for the agent to reconnect.
/// 8. Verify the serial is still the new value (new cert in use).
#[tokio::test]
#[ignore = "requires Docker"]
async fn agent_cert_renewal_via_resolver_keeps_session_alive() {
    let harness = AgentControllerHarness::start_with(HarnessOptions {
        trust_domain: String::new(),
    })
    .await;

    let agent = harness.spawn_agent("agent-1").await;
    agent.wait_for_connected().await;

    let before_serial = agent.cert_serial_number().await;

    harness
        .controller
        .request_cert_renewal(agent.service_id())
        .await;

    // Hot-swap complete: cert serial changed in the controller DB.
    agent.wait_for_cert_renewed(&before_serial).await;

    // Session must still be alive — service stays Approved without reconnecting.
    harness
        .controller
        .wait_for_service_approved(agent.service_id())
        .await;

    // Force a new TLS handshake by closing the WebSocket.
    harness
        .controller
        .disconnect_service(agent.service_id())
        .await;

    // Agent reconnects automatically; wait for it to be Approved again.
    harness
        .controller
        .wait_for_service_approved(agent.service_id())
        .await;

    let after_serial = agent.cert_serial_number().await;
    assert_ne!(
        before_serial, after_serial,
        "cert serial must differ after renewal and reconnect"
    );
}
