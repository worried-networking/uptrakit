//! End-to-end: Agent CSR → Controller signs (SPIFFE SAN preserved) → Agent
//! reconnects → Controller extracts service identity via SPIFFE SAN.
//!
//! Requires: `AgentControllerHarness` harness (not yet implemented) + Docker.
//!
//! Run when harness is ready:
//! ```sh
//! docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
//! cargo test -p uptrakit-integration-tests spiffe_identity -- --ignored --nocapture
//! ```

/// Verify that an Agent enrolls with a SPIFFE URI SAN in its CSR, the Controller
/// signs it preserving the SAN, and the Controller can extract identity via the
/// SPIFFE SAN on reconnect.
///
/// Blocked on `AgentControllerHarness` infrastructure.
#[tokio::test]
#[ignore = "Requires AgentControllerHarness (not yet implemented) + Docker"]
async fn agent_enrolls_and_authenticates_via_spiffe_san() {
    // Stub — blocked on AgentControllerHarness (not yet implemented).
    //
    // Intended sequence once harness is available:
    //   1. harness = AgentControllerHarness::start_with(HarnessOptions {
    //          trust_domain: "controller.test.local".into(), ..Default::default()
    //      }).await
    //   2. agent = harness.spawn_agent("agent-spiffe").await
    //   3. agent.wait_for_connected().await
    //   4. cert_pem = agent.current_cert_pem()
    //   5. Parse cert, extract SPIFFE URI SAN
    //   6. assert spiffe_uri == "spiffe://controller.test.local/service/<service_id>"
    //   7. identity = harness.controller.identity_of(agent.service_id()).await
    //   8. assert identity.service_id == agent.service_id()
}

/// Verify that a CSR embedding a SPIFFE URI for the wrong trust domain is
/// rejected by the Controller during signing.
///
/// Blocked on `AgentControllerHarness` infrastructure.
#[tokio::test]
#[ignore = "Requires AgentControllerHarness (not yet implemented) + Docker"]
async fn agent_with_wrong_trust_domain_csr_rejected() {
    // Stub — blocked on AgentControllerHarness (not yet implemented).
    //
    // Intended sequence once harness is available:
    //   1. harness = AgentControllerHarness::start_with(HarnessOptions {
    //          trust_domain: "controller.test.local".into(), ..Default::default()
    //      }).await
    //   2. agent = harness.spawn_agent_with_csr_override("agent-bad", |params| {
    //          params.set_spiffe_uri("spiffe://evil.example/service/{id}");
    //      }).await
    //   3. outcome = agent.wait_for_enrollment_result().await
    //   4. assert matches!(outcome, EnrollmentOutcome::Rejected { .. })
}
