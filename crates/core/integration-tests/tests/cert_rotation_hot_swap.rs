//! End-to-end verification that Agent cert renewal via the resolver hot-swap
//! path keeps the existing WebSocket session alive, then presents the new cert
//! on the next TLS handshake.
//!
//! Requires: `AgentControllerHarness` harness (not yet implemented) + Docker.
//!
//! Run when harness is ready:
//! ```sh
//! cargo test -p uptrakit-integration-tests cert_rotation_hot_swap -- --ignored --nocapture
//! ```

/// Verify that a cert renewal via the resolver hot-swap path keeps the session
/// alive and the new cert is presented on the next TLS handshake.
///
/// This test is blocked on `AgentControllerHarness` infrastructure.
/// When the harness exists, replace `todo!()` with the full assertion sequence.
#[tokio::test]
#[ignore = "Requires AgentControllerHarness (not yet implemented) + Docker"]
async fn agent_cert_renewal_via_resolver_keeps_session_alive() {
    // TODO: implement when AgentControllerHarness is available.
    //
    // Intended sequence:
    //   1. harness.spawn_agent("agent-1").wait_for_connected()
    //   2. capture session_id_before = agent.current_tls_session_id()
    //   3. harness.controller.request_cert_renewal(agent.service_id())
    //   4. agent.wait_for_cert_renewed()
    //   5. assert session_id_before == session_id_after  (no reconnect)
    //   6. agent.force_handshake()
    //   7. assert presented_cert == agent.current_cert_pem()
    todo!("AgentControllerHarness not yet implemented");
}
