//! Handlers for workload claim protocol messages.
//!
//! Services send `WorkloadClaim` to request exclusive ownership of config keys,
//! and `WorkloadRelease` to voluntarily relinquish them. The controller
//! arbitrates and responds with `WorkloadClaimResult`.

use std::sync::Arc;

use uptrakit_internal_wire::{
    ControllerMessage, WorkloadClaimAnnouncementPayload, WorkloadClaimPayload,
    WorkloadClaimResultPayload, WorkloadReleasePayload,
};

use crate::app_state::AppState;

use super::shared_types::ProcessorResponse;

/// Handle a `WorkloadClaim` message (full replacement semantics).
///
/// The claim map contains `config_key → tenant_id`. The controller diffs
/// against the service's current grants, grants unclaimed keys, rejects
/// already-claimed ones, and releases keys that the service no longer desires.
///
/// After granting, the controller:
/// 1. Sends `WorkloadClaimResult` to the service
/// 2. Publishes `WorkloadClaimAnnouncement` to NATS for cross-controller sync
/// 3. Pushes initial `SoftwareStates` and `HostConnectivityUpdated` for any
///    newly served tenants
#[tracing::instrument(skip_all, fields(%service_id))]
pub(super) async fn handle_workload_claim(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: WorkloadClaimPayload,
) -> ProcessorResponse {
    let controller_id = state.controller_id;
    let cr = &state.workload_claim_registry;

    let result = cr.try_claim(service_id, controller_id, payload.claims);

    // Build the result message.
    let claim_result =
        WorkloadClaimResultPayload::new(result.granted.clone(), result.rejected.clone());

    // Publish announcement to NATS for cross-controller sync.
    if !result.granted.is_empty() || !result.released.is_empty() {
        let claimed_at = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let claimed: std::collections::BTreeMap<String, uuid::Uuid> = result
            .granted
            .iter()
            .filter_map(|k| cr.tenant_for_key(k).map(|tid| (k.clone(), tid)))
            .collect();
        let released: std::collections::BTreeSet<String> =
            result.released.keys().cloned().collect();
        let announcement = WorkloadClaimAnnouncementPayload::new(
            service_id,
            controller_id,
            claimed,
            released,
            claimed_at,
        );
        state
            .notification
            .notification_service
            .publish_controller_event(ControllerMessage::WorkloadClaimAnnouncement(announcement))
            .await;
    }

    // Push initial state for newly served tenants.
    let new_tenants = result.new_tenants();
    if !new_tenants.is_empty() {
        let db = state.db();
        for tid in &new_tenants {
            state
                .notification
                .notification_service
                .push_software_states_paginated_for_tenant(db, *tid)
                .await;
            state
                .notification
                .notification_service
                .push_connected_agent_states_for_tenant(db, *tid)
                .await;
        }
    }

    ProcessorResponse::reply(ControllerMessage::WorkloadClaimResult(claim_result))
}

/// Handle a `WorkloadRelease` message (voluntary release of specific keys).
#[tracing::instrument(skip_all, fields(%service_id))]
pub(super) async fn handle_workload_release(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: WorkloadReleasePayload,
) -> ProcessorResponse {
    let controller_id = state.controller_id;
    let cr = &state.workload_claim_registry;

    let released = cr.release_keys(service_id, &payload.keys);

    if !released.is_empty() {
        // Publish release announcement to NATS.
        let claimed_at = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let announcement = WorkloadClaimAnnouncementPayload::new(
            service_id,
            controller_id,
            std::collections::BTreeMap::new(),
            released.keys().cloned().collect(),
            claimed_at,
        );
        state
            .notification
            .notification_service
            .publish_controller_event(ControllerMessage::WorkloadClaimAnnouncement(announcement))
            .await;

        // Proactive re-grant for released keys.
        let released_keys: std::collections::BTreeSet<String> = released.keys().cloned().collect();
        let re_grantable = cr.find_pending_desires_for_keys(&released_keys);
        for (svc_id, desired) in re_grantable {
            let result = cr.try_claim(svc_id, controller_id, desired);
            if !result.granted.is_empty() {
                let new_tenants = result.new_tenants();
                let claim_result =
                    WorkloadClaimResultPayload::new(result.granted.clone(), result.rejected);
                state
                    .notification
                    .notification_service
                    .send(
                        &svc_id,
                        ControllerMessage::WorkloadClaimResult(claim_result),
                    )
                    .await;
                // Announce re-grants via NATS.
                let ts = time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default();
                let claimed: std::collections::BTreeMap<String, uuid::Uuid> = result
                    .granted
                    .iter()
                    .filter_map(|k| cr.tenant_for_key(k).map(|tid| (k.clone(), tid)))
                    .collect();
                let ann = WorkloadClaimAnnouncementPayload::new(
                    svc_id,
                    controller_id,
                    claimed,
                    std::collections::BTreeSet::new(),
                    ts,
                );
                state
                    .notification
                    .notification_service
                    .publish_controller_event(ControllerMessage::WorkloadClaimAnnouncement(ann))
                    .await;
                // Push initial state for newly served tenants.
                let db = state.db();
                for tid in &new_tenants {
                    state
                        .notification
                        .notification_service
                        .push_software_states_paginated_for_tenant(db, *tid)
                        .await;
                    state
                        .notification
                        .notification_service
                        .push_connected_agent_states_for_tenant(db, *tid)
                        .await;
                }
            }
        }
    }

    ProcessorResponse::cont()
}

/// Release all claims held by a service on disconnect and announce via NATS.
///
/// Called from the cleanup path when an authenticated session ends.
pub(super) async fn release_all_claims_on_disconnect(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
) {
    let controller_id = state.controller_id;
    let cr = &state.workload_claim_registry;

    let released = cr.release(service_id);
    if released.is_empty() {
        return;
    }

    tracing::info!(
        %service_id,
        released_keys = released.len(),
        "released workload claims on disconnect"
    );

    // Publish release announcement to NATS.
    let claimed_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let announcement = WorkloadClaimAnnouncementPayload::new(
        service_id,
        controller_id,
        std::collections::BTreeMap::new(),
        released.keys().cloned().collect(),
        claimed_at,
    );
    state
        .notification
        .notification_service
        .publish_controller_event(ControllerMessage::WorkloadClaimAnnouncement(announcement))
        .await;

    // Proactive re-grant for released keys.
    let released_keys: std::collections::BTreeSet<String> = released.keys().cloned().collect();
    let re_grantable = cr.find_pending_desires_for_keys(&released_keys);
    for (svc_id, desired) in re_grantable {
        let result = cr.try_claim(svc_id, controller_id, desired);
        if !result.granted.is_empty() {
            let new_tenants = result.new_tenants();
            let claim_result =
                WorkloadClaimResultPayload::new(result.granted.clone(), result.rejected);
            state
                .notification
                .notification_service
                .send(
                    &svc_id,
                    ControllerMessage::WorkloadClaimResult(claim_result),
                )
                .await;
            let ts = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default();
            let claimed: std::collections::BTreeMap<String, uuid::Uuid> = result
                .granted
                .iter()
                .filter_map(|k| cr.tenant_for_key(k).map(|tid| (k.clone(), tid)))
                .collect();
            let ann = WorkloadClaimAnnouncementPayload::new(
                svc_id,
                controller_id,
                claimed,
                std::collections::BTreeSet::new(),
                ts,
            );
            state
                .notification
                .notification_service
                .publish_controller_event(ControllerMessage::WorkloadClaimAnnouncement(ann))
                .await;
            let db = state.db();
            for tid in &new_tenants {
                state
                    .notification
                    .notification_service
                    .push_software_states_paginated_for_tenant(db, *tid)
                    .await;
                state
                    .notification
                    .notification_service
                    .push_connected_agent_states_for_tenant(db, *tid)
                    .await;
            }
        }
    }
}
