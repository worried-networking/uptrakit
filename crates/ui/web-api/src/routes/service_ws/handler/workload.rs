//! Handlers for workload claim protocol messages.
//!
//! Services send `WorkloadClaim` to request exclusive ownership of config keys,
//! and `WorkloadRelease` to voluntarily relinquish them. The controller
//! arbitrates and responds with `WorkloadClaimResult`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sea_orm::EntityTrait;
use uptrakit_audit_log::{AuditActionType, AuditEntry, AuditOutcome};
use uptrakit_internal_wire::{
    ControllerMessage, WorkloadClaimAnnouncementPayload, WorkloadClaimPayload,
    WorkloadClaimResultPayload, WorkloadReleasePayload,
};
use uptrakit_shared_db::entity::{service, system_service};

use crate::app_state::AppState;

use super::shared_types::ProcessorResponse;

fn workload_scope_for_tenants(
    tenant_ids: impl IntoIterator<Item = uuid::Uuid>,
) -> Option<uuid::Uuid> {
    let unique_tenants: BTreeSet<_> = tenant_ids.into_iter().collect();
    (unique_tenants.len() == 1).then(|| *unique_tenants.first().expect("single tenant present"))
}

fn workload_target_display(
    friendly_name: &str,
    hostname: &str,
    service_app_name: Option<&str>,
    service_id: uuid::Uuid,
) -> String {
    if !friendly_name.is_empty() {
        friendly_name.to_string()
    } else if !hostname.is_empty() {
        hostname.to_string()
    } else if let Some(service_app_name) = service_app_name.filter(|value| !value.is_empty()) {
        service_app_name.to_string()
    } else {
        service_id.to_string()
    }
}

async fn resolve_workload_audit_identity(
    state: &AppState,
    service_id: uuid::Uuid,
) -> (Option<String>, String) {
    match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(service)) => (
            service
                .service_app_name
                .clone()
                .and_then(|value| (!value.is_empty()).then_some(value)),
            workload_target_display(
                &service.friendly_name,
                &service.hostname,
                service.service_app_name.as_deref(),
                service_id,
            ),
        ),
        Ok(None) => match system_service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(service)) => (
                service
                    .service_app_name
                    .clone()
                    .and_then(|value| (!value.is_empty()).then_some(value)),
                workload_target_display(
                    &service.friendly_name,
                    &service.hostname,
                    service.service_app_name.as_deref(),
                    service_id,
                ),
            ),
            Ok(None) => (None, service_id.to_string()),
            Err(error) => {
                tracing::warn!(
                    %service_id,
                    error = %error,
                    "failed to resolve system service identity for workload audit"
                );
                (None, service_id.to_string())
            }
        },
        Err(error) => {
            tracing::warn!(
                %service_id,
                error = %error,
                "failed to resolve tenant service identity for workload audit"
            );
            (None, service_id.to_string())
        }
    }
}

async fn emit_workload_audit_event(
    state: &Arc<AppState>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    service_id: uuid::Uuid,
    tenant_scope: Option<uuid::Uuid>,
    outcome: AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_display, target_display) = resolve_workload_audit_identity(state, service_id).await;
    let mut builder = AuditEntry::builder(action_type)
        .actor_service(service_id)
        .actor_display_opt(actor_display)
        .target("service", service_id.to_string(), Some(target_display))
        .outcome(outcome)
        .details(details);
    builder = if let Some(tenant_id) = tenant_scope {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };
    match builder.build() {
        Ok(entry) => state.audit_emitter.emit_best_effort(entry),
        Err(error) => tracing::warn!(
            %service_id,
            action_type = %action_type,
            error = %error,
            "failed to build workload semantic audit entry"
        ),
    }
}

struct WorkloadAuditCtx<'a> {
    state: &'a Arc<AppState>,
    service_id: uuid::Uuid,
}

async fn emit_workload_claim_audit_event(
    ctx: WorkloadAuditCtx<'_>,
    requested_claims: &BTreeMap<String, uuid::Uuid>,
    granted: &BTreeSet<String>,
    rejected: &BTreeSet<String>,
    released: &BTreeMap<String, uuid::Uuid>,
    claim_source: &'static str,
    triggering_service_id: Option<uuid::Uuid>,
) {
    if granted.is_empty() && rejected.is_empty() {
        return;
    }

    let outcome = if rejected.is_empty() {
        AuditOutcome::Success
    } else if granted.is_empty() {
        AuditOutcome::Denied
    } else {
        AuditOutcome::Partial
    };

    let tenant_scope = workload_scope_for_tenants(requested_claims.values().copied());
    let mut details = serde_json::json!({
        "claim_source": claim_source,
        "requested_key_count": requested_claims.len(),
        "granted_key_count": granted.len(),
        "rejected_key_count": rejected.len(),
        "released_key_count": released.len(),
        "tenant_count": requested_claims.values().copied().collect::<BTreeSet<_>>().len(),
    });
    if !rejected.is_empty() {
        details["reason_code"] = serde_json::Value::String("already_claimed".to_string());
    }
    if let Some(triggering_service_id) = triggering_service_id {
        details["triggering_service_id"] =
            serde_json::Value::String(triggering_service_id.to_string());
    }

    emit_workload_audit_event(
        ctx.state,
        AuditActionType::SERVICE_WORKLOAD_CLAIM,
        ctx.service_id,
        tenant_scope,
        outcome,
        details,
    )
    .await;
}

async fn emit_workload_release_audit_event(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    released: &BTreeMap<String, uuid::Uuid>,
    release_source: &'static str,
) {
    if released.is_empty() {
        return;
    }

    let tenant_scope = workload_scope_for_tenants(released.values().copied());
    let details = serde_json::json!({
        "release_source": release_source,
        "released_key_count": released.len(),
        "tenant_count": released.values().copied().collect::<BTreeSet<_>>().len(),
    });
    emit_workload_audit_event(
        state,
        AuditActionType::SERVICE_WORKLOAD_RELEASE,
        service_id,
        tenant_scope,
        AuditOutcome::Success,
        details,
    )
    .await;
}

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
    let WorkloadClaimPayload { claims, .. } = payload;

    let result = cr.try_claim(service_id, controller_id, claims.clone());

    // Build the result message.
    let claim_result =
        WorkloadClaimResultPayload::new(result.granted.clone(), result.rejected.clone());

    emit_workload_claim_audit_event(
        WorkloadAuditCtx { state, service_id },
        &claims,
        &result.granted,
        &result.rejected,
        &result.released,
        "request",
        None,
    )
    .await;
    emit_workload_release_audit_event(state, service_id, &result.released, "claim_replacement")
        .await;

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
        emit_workload_release_audit_event(state, service_id, &released, "request").await;

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
                emit_workload_claim_audit_event(
                    WorkloadAuditCtx {
                        state,
                        service_id: svc_id,
                    },
                    &result
                        .granted
                        .iter()
                        .filter_map(|key| {
                            cr.tenant_for_key(key)
                                .map(|tenant_id| (key.clone(), tenant_id))
                        })
                        .chain(result.rejected.iter().filter_map(|key| {
                            released
                                .get(key)
                                .copied()
                                .map(|tenant_id| (key.clone(), tenant_id))
                        }))
                        .collect(),
                    &result.granted,
                    &result.rejected,
                    &BTreeMap::new(),
                    "regrant",
                    Some(service_id),
                )
                .await;
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

    emit_workload_release_audit_event(state, service_id, &released, "disconnect").await;

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
            emit_workload_claim_audit_event(
                WorkloadAuditCtx {
                    state,
                    service_id: svc_id,
                },
                &result
                    .granted
                    .iter()
                    .filter_map(|key| {
                        cr.tenant_for_key(key)
                            .map(|tenant_id| (key.clone(), tenant_id))
                    })
                    .chain(result.rejected.iter().filter_map(|key| {
                        released
                            .get(key)
                            .copied()
                            .map(|tenant_id| (key.clone(), tenant_id))
                    }))
                    .collect(),
                &result.granted,
                &result.rejected,
                &BTreeMap::new(),
                "regrant",
                Some(service_id),
            )
            .await;
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

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;

    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
    use uptrakit_internal_wire::ControllerMessage;
    use uptrakit_shared_db::entity::{audit_log, service};

    async fn insert_service_row(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        service_id: uuid::Uuid,
        service_app_name: &str,
    ) {
        let now = time::OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("host-{service_id}")),
            friendly_name: Set(format!("Service {service_id}")),
            ip_address: Set(Some("10.0.0.1".to_string())),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(Some(service_app_name.to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert service");
    }

    async fn wait_for_tenant_audit_row(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
        actor_id: uuid::Uuid,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .filter(audit_log::Column::ActorId.eq(actor_id))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query tenant audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type} and actor {actor_id}");
    }

    fn assert_claim_reply(
        response: &ProcessorResponse,
        expected_granted: &[&str],
        expected_rejected: &[&str],
    ) {
        let [ControllerMessage::WorkloadClaimResult(result)] = response.replies.as_slice() else {
            panic!("expected exactly one WorkloadClaimResult reply");
        };
        assert_eq!(
            result.granted,
            expected_granted
                .iter()
                .map(|value| value.to_string())
                .collect()
        );
        assert_eq!(
            result.rejected,
            expected_rejected
                .iter()
                .map(|value| value.to_string())
                .collect()
        );
    }

    #[tokio::test]
    async fn handle_workload_claim_emits_success_audit_entry() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        insert_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;

        let claim_key = format!("clients.{}", uuid::Uuid::now_v7());
        let response = handle_workload_claim(
            &state,
            service_id,
            WorkloadClaimPayload::new(BTreeMap::from([(claim_key.clone(), tenant_id)])),
        )
        .await;

        assert_claim_reply(&response, &[claim_key.as_str()], &[]);

        let row =
            wait_for_tenant_audit_row(&db, AuditActionType::SERVICE_WORKLOAD_CLAIM, service_id)
                .await;
        assert_eq!(row.outcome, AuditOutcome::Success.as_str());
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
        let details = row
            .details_json
            .expect("claim audit should include details");
        assert_eq!(details["claim_source"], "request");
        assert_eq!(details["requested_key_count"], 1);
        assert_eq!(details["granted_key_count"], 1);
        assert_eq!(details["rejected_key_count"], 0);
    }

    #[tokio::test]
    async fn handle_workload_claim_emits_denied_audit_entry_when_key_is_already_claimed() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let owner_service_id = uuid::Uuid::now_v7();
        let rejected_service_id = uuid::Uuid::now_v7();
        insert_service_row(&db, tenant_id, owner_service_id, "uptrakit-mqtt").await;
        insert_service_row(&db, tenant_id, rejected_service_id, "uptrakit-mqtt").await;

        let claim_key = format!("clients.{}", uuid::Uuid::now_v7());
        let _ = handle_workload_claim(
            &state,
            owner_service_id,
            WorkloadClaimPayload::new(BTreeMap::from([(claim_key.clone(), tenant_id)])),
        )
        .await;

        let response = handle_workload_claim(
            &state,
            rejected_service_id,
            WorkloadClaimPayload::new(BTreeMap::from([(claim_key.clone(), tenant_id)])),
        )
        .await;

        assert_claim_reply(&response, &[], &[claim_key.as_str()]);

        let row = wait_for_tenant_audit_row(
            &db,
            AuditActionType::SERVICE_WORKLOAD_CLAIM,
            rejected_service_id,
        )
        .await;
        assert_eq!(row.outcome, AuditOutcome::Denied.as_str());
        let details = row
            .details_json
            .expect("claim denial audit should include details");
        assert_eq!(details["claim_source"], "request");
        assert_eq!(details["reason_code"], "already_claimed");
        assert_eq!(details["granted_key_count"], 0);
        assert_eq!(details["rejected_key_count"], 1);
    }

    #[tokio::test]
    async fn handle_workload_release_emits_release_and_regrant_audit_entries() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let owner_service_id = uuid::Uuid::now_v7();
        let waiting_service_id = uuid::Uuid::now_v7();
        insert_service_row(&db, tenant_id, owner_service_id, "uptrakit-mqtt").await;
        insert_service_row(&db, tenant_id, waiting_service_id, "uptrakit-mqtt").await;

        let claim_key = format!("clients.{}", uuid::Uuid::now_v7());
        let _ = handle_workload_claim(
            &state,
            owner_service_id,
            WorkloadClaimPayload::new(BTreeMap::from([(claim_key.clone(), tenant_id)])),
        )
        .await;
        let _ = handle_workload_claim(
            &state,
            waiting_service_id,
            WorkloadClaimPayload::new(BTreeMap::from([(claim_key.clone(), tenant_id)])),
        )
        .await;

        let response = handle_workload_release(
            &state,
            owner_service_id,
            WorkloadReleasePayload::new(BTreeSet::from([claim_key])),
        )
        .await;

        assert!(response.replies.is_empty());

        let release_row = wait_for_tenant_audit_row(
            &db,
            AuditActionType::SERVICE_WORKLOAD_RELEASE,
            owner_service_id,
        )
        .await;
        assert_eq!(release_row.outcome, AuditOutcome::Success.as_str());
        let release_details = release_row
            .details_json
            .expect("release audit should include details");
        assert_eq!(release_details["release_source"], "request");
        assert_eq!(release_details["released_key_count"], 1);

        let regrant_row = wait_for_tenant_audit_row(
            &db,
            AuditActionType::SERVICE_WORKLOAD_CLAIM,
            waiting_service_id,
        )
        .await;
        assert_eq!(regrant_row.outcome, AuditOutcome::Success.as_str());
        let regrant_details = regrant_row
            .details_json
            .expect("regrant audit should include details");
        assert_eq!(regrant_details["claim_source"], "regrant");
        assert_eq!(
            regrant_details["triggering_service_id"],
            owner_service_id.to_string()
        );
        assert_eq!(regrant_details["granted_key_count"], 1);
    }

    #[tokio::test]
    async fn release_all_claims_on_disconnect_emits_release_audit_entry() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        insert_service_row(&db, tenant_id, service_id, "uptrakit-mqtt").await;

        let claim_key = format!("clients.{}", uuid::Uuid::now_v7());
        let _ = handle_workload_claim(
            &state,
            service_id,
            WorkloadClaimPayload::new(BTreeMap::from([(claim_key, tenant_id)])),
        )
        .await;

        release_all_claims_on_disconnect(&state, service_id).await;

        let row =
            wait_for_tenant_audit_row(&db, AuditActionType::SERVICE_WORKLOAD_RELEASE, service_id)
                .await;
        assert_eq!(row.outcome, AuditOutcome::Success.as_str());
        let details = row
            .details_json
            .expect("disconnect release should include details");
        assert_eq!(details["release_source"], "disconnect");
        assert_eq!(details["released_key_count"], 1);
    }
}
