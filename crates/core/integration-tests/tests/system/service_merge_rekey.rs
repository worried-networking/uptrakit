//! End-to-end Docker integration test for the service merge re-key flow.
//!
//! # Flow
//!
//! 1. Boot a controller in Docker.
//! 2. Start a first agent-ssh with the bootstrap enrollment token — it becomes
//!    **Approved** (the merge target).
//! 3. Start a second agent-ssh **without** an enrollment token — it enrolls and
//!    stays **Pending** (the merge source).  The container waits for the
//!    "waiting for approval..." SDK log line confirming the service is visible.
//! 4. Call `POST /api/v1/services/{target_id}/merge` with `source_id`.
//!    Assert 200 OK.
//! 5. Restart the source agent-ssh container (stop + start, filesystem preserved).
//!    The agent reconnects using its persisted enrollment secret; the controller
//!    follows the `service_merge_redirect` row and re-binds the secret to
//!    `target_id`.
//! 6. Wait for a `auth.service.rekey_resolved` audit log entry, confirming the
//!    controller performed the re-key lookup.
//! 7. Assert the **target** service is still present and Approved.
//! 8. Assert the **source** service is gone from the active list (deactivated).

use std::time::Duration;

use uptrakit_openapi_client::types::services::ServiceStatus;

use crate::helpers::api_client::ApiClient;
use crate::helpers::containers::{ControllerContainer, ServiceContainer, test_network_name};

/// End-to-end test for the service merge re-key flow.
///
/// Strategy for Approved (disconnected) target + Pending (live) source:
///
/// 1. First agent-ssh enrolls with the bootstrap token → auto-Approved (target).
///    Then we **drop** that container so its WebSocket connection closes.  The
///    merge route rejects an attempt to merge a *connected* target, so the
///    target must be offline at merge time.
/// 2. Second agent-ssh enrolls without a token → stays Pending (source).
/// 3. We merge source into the now-disconnected target.
/// 4. We restart the source container; the agent reconnects with its persisted
///    enrollment secret, the controller follows the redirect row, and emits
///    `auth.service.rekey_resolved`.
///
/// The controller always has 2 embedded services.  After step 1 we have 3
/// total (the target row persists in the DB even after the container is
/// stopped).  After step 2 we have 4; after the merge the source row is
/// deactivated and we return to 3.
#[tokio::test]
#[ignore = "System integration test (requires uptrakit-test:latest Docker image). Run: cargo test -p uptrakit-integration-tests --test system service_merge_rekey -- --ignored"]
async fn service_merge_rekey_end_to_end() {
    let network = test_network_name();
    let controller = ControllerContainer::start(&network).await;

    let mut client = ApiClient::new(controller.host_port());
    client.wait_for_ready(Duration::from_secs(30)).await;
    client
        .register_and_login_with_token(controller.registration_token())
        .await;

    // -----------------------------------------------------------------------
    // Step 1: Enroll the first agent-ssh (Approved — merge target).
    //
    // After capturing the target UUID we drop the container so the controller
    // sees it as disconnected.  The merge route rejects a connected target to
    // prevent a split-brain scenario, so the target must be offline first.
    // -----------------------------------------------------------------------
    let target_id = {
        let agent_target =
            ServiceContainer::start_agent_ssh(&network, controller.container_name()).await;

        // Wait for the 3 services: 2 embedded + 1 external (approved agent-ssh).
        let services_after_target = client
            .wait_for_service_count(3, Duration::from_secs(60))
            .await;

        let target_service = services_after_target
            .iter()
            .find(|s| !s.is_embedded && s.status == ServiceStatus::Approved)
            .expect("expected one external Approved service (target)");
        let id = target_service.id;

        // Drop the container; the WS connection is torn down.
        drop(agent_target);

        // Give the controller a moment to notice the TCP close and remove the
        // connection from its registry before we issue the merge call.
        tokio::time::sleep(Duration::from_secs(3)).await;

        id
    };

    // -----------------------------------------------------------------------
    // Step 2: Enroll the second agent-ssh WITHOUT a token (Pending — merge source).
    // -----------------------------------------------------------------------
    let agent_source =
        ServiceContainer::start_agent_ssh_pending(&network, controller.container_name()).await;

    // Wait for the 4th service to appear (source is Pending, still visible).
    let services_after_source = client
        .wait_for_service_count(4, Duration::from_secs(60))
        .await;

    let source_service = services_after_source
        .iter()
        .find(|s| !s.is_embedded && s.status == ServiceStatus::Pending)
        .expect("expected one external Pending service (source)");
    let source_id = source_service.id;

    assert_ne!(
        target_id, source_id,
        "target and source must be distinct services"
    );

    // -----------------------------------------------------------------------
    // Step 3: Merge source into target via REST API.
    // -----------------------------------------------------------------------
    let merged = client.merge_service(target_id, source_id).await;
    assert_eq!(
        merged.id, target_id,
        "merge response must carry the target service id"
    );
    assert_eq!(
        merged.status,
        ServiceStatus::Approved,
        "target must remain Approved after merge"
    );

    // -----------------------------------------------------------------------
    // Step 4: Restart the source agent-ssh container.
    //
    // The container filesystem is preserved (stop + start, not remove +
    // recreate).  The agent reconnects using its persisted enrollment secret.
    // The controller follows the redirect row written by the merge and re-binds
    // the secret to target_id.
    // -----------------------------------------------------------------------
    agent_source.restart().await;

    // -----------------------------------------------------------------------
    // Step 5: Assert the `auth.service.rekey_resolved` audit entry appears.
    //
    // The controller emits this event when it successfully resolves a
    // `service_merge_redirect` lookup — i.e. when the restarted source agent
    // re-authenticates and is redirected to the target.  We poll the audit log
    // API with the action_type filter; any entry confirms the re-key path was
    // exercised.
    // -----------------------------------------------------------------------
    let rekey_audit = client
        .wait_for_audit_log_entry("auth.service.rekey_resolved", Duration::from_secs(90))
        .await;

    // The audit entry details should contain both source and target UUIDs.
    let details = rekey_audit
        .details_json
        .expect("auth.service.rekey_resolved must carry details_json");
    let details_str = details.to_string();
    assert!(
        details_str.contains(&source_id.to_string()),
        "audit details must reference source_id ({source_id}), got: {details_str}"
    );
    assert!(
        details_str.contains(&target_id.to_string()),
        "audit details must reference target_id ({target_id}), got: {details_str}"
    );

    // -----------------------------------------------------------------------
    // Step 6: Assert target is still Approved, source is gone.
    // -----------------------------------------------------------------------
    let services_final = client
        .wait_for_service_count(3, Duration::from_secs(30))
        .await;

    // Target must be present and Approved.
    let final_target = services_final
        .iter()
        .find(|s| s.id == target_id)
        .expect("target service must still be present after merge");
    assert_eq!(
        final_target.status,
        ServiceStatus::Approved,
        "target must remain Approved after re-key"
    );

    // Source must be absent (deactivated_at is set, list excludes it).
    assert!(
        !services_final.iter().any(|s| s.id == source_id),
        "source service must be deactivated (absent from list) after merge"
    );
}
