//! End-to-end: Agent CSR → Controller signs (SPIFFE SAN preserved) → Agent
//! reconnects → Controller extracts service identity via SPIFFE SAN.
//!
//! Run:
//! ```sh
//! docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
//! cargo test -p uptrakit-integration-tests spiffe_identity -- --ignored --nocapture
//! ```
// Shared helpers include members not used by this test (e.g. start_scheduler).
#![expect(
    dead_code,
    reason = "shared test helpers include members not exercised by this test binary"
)]

mod helpers;

#[path = "helpers/agent_controller_harness.rs"]
mod agent_controller_harness;

use crate::agent_controller_harness::{AgentControllerHarness, HarnessOptions};

/// Verify that an Agent enrolls with a SPIFFE URI SAN in its CSR, the Controller
/// signs it preserving the SAN, and the Controller returns the SPIFFE identity
/// in the service detail response.
#[tokio::test]
#[ignore = "requires Docker"]
async fn agent_enrolls_and_authenticates_via_spiffe_san() {
    let harness = AgentControllerHarness::start_with(HarnessOptions {
        trust_domain: "controller.test.local".into(),
    })
    .await;

    let agent = harness.spawn_agent("agent-spiffe").await;
    agent.wait_for_connected().await;

    let identity = harness.controller.identity_of(agent.service_id()).await;
    let spiffe_id = identity
        .spiffe_id
        .expect("SPIFFE ID must be present when trust domain is configured");

    assert_eq!(
        spiffe_id,
        format!(
            "spiffe://controller.test.local/service/{}",
            agent.service_id()
        ),
        "SPIFFE URI must match the configured trust domain and service ID"
    );
}

// The wrong-trust-domain rejection path is tested at the unit level in
// `controller-runtime/src/cert_signer.rs::spiffe_san_wrong_trust_domain_rejected`.
// No Docker integration test is needed: the rejection happens synchronously
// inside `RcgenAgentCertSigner::sign_agent_csr` before any network traffic.
