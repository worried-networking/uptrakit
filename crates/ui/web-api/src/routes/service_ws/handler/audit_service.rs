//! Service-lifecycle audit helpers.
//!
//! Classification, scope routing, validation and emit fns for the
//! service audit event flow (enrollment, certificate issue/renew, and
//! forwarded agent audit events).

use crate::AppState;
use sea_orm::EntityTrait;
use std::sync::Arc;
use uptrakit_shared_db::entity::{service, system_service as sys_svc_entity};
use uptrakit_wire::AuditEventPayload;
use uptrakit_wire::limits::{MAX_LONG_STRING_LEN, MAX_SHORT_STRING_LEN};

const SYSTEM_SERVICE_AUDIT_ACTIONS: &[uptrakit_audit_log::RegisteredAuditAction] =
    &[uptrakit_audit_log::AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP];
const TENANT_SERVICE_AUDIT_ACTIONS: &[uptrakit_audit_log::RegisteredAuditAction] = &[
    uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
    uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STARTED,
    uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_STARTED,
    uptrakit_audit_log::AuditActionType::HOST_UPDATE,
];
const SERVICE_BOUND_AUDIT_ACTIONS: &[uptrakit_audit_log::RegisteredAuditAction] = &[
    uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_ISSUE,
    uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
    uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_GATE,
    uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_MACHINE_ID_VALIDATE,
    uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_FREEZE_APPLY,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuditEventScope {
    TenantOnly,
    ServiceBound,
    SystemOnly,
}

fn audit_event_scope(action_type: &uptrakit_audit_log::AuditActionType) -> Option<AuditEventScope> {
    if TENANT_SERVICE_AUDIT_ACTIONS
        .iter()
        .any(|registered| registered.as_str() == action_type.as_str())
    {
        Some(AuditEventScope::TenantOnly)
    } else if SERVICE_BOUND_AUDIT_ACTIONS
        .iter()
        .any(|registered| registered.as_str() == action_type.as_str())
    {
        Some(AuditEventScope::ServiceBound)
    } else if SYSTEM_SERVICE_AUDIT_ACTIONS
        .iter()
        .any(|registered| registered.as_str() == action_type.as_str())
    {
        Some(AuditEventScope::SystemOnly)
    } else {
        None
    }
}

fn validate_audit_event_payload(
    payload: &AuditEventPayload,
) -> Result<
    (
        uptrakit_audit_log::AuditActionType,
        uptrakit_audit_log::AuditOutcome,
        AuditEventScope,
        Option<serde_json::Value>,
    ),
    String,
> {
    if payload.action_type.is_empty() {
        return Err("action_type must not be empty".to_string());
    }
    if payload.action_type.len() > MAX_SHORT_STRING_LEN {
        return Err(format!("action_type exceeds {MAX_SHORT_STRING_LEN} bytes"));
    }
    let action_type = payload
        .action_type
        .parse::<uptrakit_audit_log::AuditActionType>()
        .map_err(|error| error.to_string())?;
    let scope = audit_event_scope(&action_type)
        .ok_or_else(|| format!("unsupported audit action_type: {}", action_type.as_str()))?;
    if payload.outcome.is_empty() {
        return Err("outcome must not be empty".to_string());
    }
    if payload.outcome.len() > MAX_SHORT_STRING_LEN {
        return Err(format!("outcome exceeds {MAX_SHORT_STRING_LEN} bytes"));
    }
    let outcome = uptrakit_audit_log::AuditOutcome::try_from(payload.outcome.as_str())
        .map_err(|_| format!("unsupported audit outcome: {}", payload.outcome))?;
    for (field, value) in [
        ("tenant_id", payload.tenant_id.as_deref()),
        ("target_type", payload.target_type.as_deref()),
        ("target_id", payload.target_id.as_deref()),
        ("target_display", payload.target_display.as_deref()),
        ("request_id", payload.request_id.as_deref()),
    ] {
        if let Some(value) = value
            && value.len() > MAX_SHORT_STRING_LEN
        {
            return Err(format!("{field} exceeds {MAX_SHORT_STRING_LEN} bytes"));
        }
    }
    let details_json = match payload.details_json.as_deref() {
        Some(details_json) => {
            if details_json.len() > MAX_LONG_STRING_LEN {
                return Err(format!("details_json exceeds {MAX_LONG_STRING_LEN} bytes"));
            }
            Some(
                serde_json::from_str::<serde_json::Value>(details_json)
                    .map_err(|error| format!("details_json is not valid JSON: {error}"))?,
            )
        }
        None => None,
    };
    Ok((action_type, outcome, scope, details_json))
}

async fn resolve_service_audit_identity(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
) -> Option<(Option<uuid::Uuid>, String)> {
    if is_system {
        match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(service)) => Some((
                None,
                service
                    .service_app_name
                    .unwrap_or_else(|| "unknown".to_string()),
            )),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    %service_id,
                    error = %error,
                    "failed to resolve system service audit identity"
                );
                None
            }
        }
    } else {
        match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(service)) => Some((
                Some(service.tenant_id),
                service
                    .service_app_name
                    .unwrap_or_else(|| "unknown".to_string()),
            )),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    %service_id,
                    error = %error,
                    "failed to resolve tenant service audit identity"
                );
                None
            }
        }
    }
}

async fn resolve_service_target_display(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
) -> String {
    if is_system {
        if let Ok(Some(service)) = sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            if !service.friendly_name.is_empty() {
                return service.friendly_name;
            }
            if !service.hostname.is_empty() {
                return service.hostname;
            }
            if let Some(service_app_name) =
                service.service_app_name.filter(|value| !value.is_empty())
            {
                return service_app_name;
            }
        }
    } else if let Ok(Some(service)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        if !service.friendly_name.is_empty() {
            return service.friendly_name;
        }
        if !service.hostname.is_empty() {
            return service.hostname;
        }
        if let Some(service_app_name) = service.service_app_name.filter(|value| !value.is_empty()) {
            return service_app_name;
        }
    }

    service_id.to_string()
}

pub(super) async fn ingest_service_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
    service_tenant_id: Option<uuid::Uuid>,
    service_app_name: Option<&str>,
    payload: AuditEventPayload,
) -> bool {
    // Reject forwarded Stateful action types before general validation.
    // Services must not emit Stateful events — those originate only from the controller.
    if let Ok(parsed_action) = payload
        .action_type
        .parse::<uptrakit_audit_log::AuditActionType>()
        && parsed_action.kind() == Some(uptrakit_audit_log::AuditActionKind::Stateful)
    {
        tracing::warn!(
            %service_id,
            action_type = %payload.action_type,
            "rejecting forwarded Stateful audit event; service-side stateful emission is not supported"
        );
        return false;
    }

    let (action_type, outcome, scope, details_json) = match validate_audit_event_payload(&payload) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                %service_id,
                action_type = %payload.action_type,
                error = %error,
                "dropping invalid service audit event"
            );
            return false;
        }
    };

    let (resolved_tenant_id, resolved_service_app_name) =
        if service_tenant_id.is_none() || service_app_name.is_none() {
            match resolve_service_audit_identity(state, service_id, is_system).await {
                Some((tenant_id, app_name)) => (
                    service_tenant_id.or(tenant_id),
                    service_app_name.map(str::to_string).unwrap_or(app_name),
                ),
                None => {
                    tracing::warn!(
                        %service_id,
                        action_type = %action_type,
                        "dropping service audit event for unknown service"
                    );
                    return false;
                }
            }
        } else {
            (
                service_tenant_id,
                service_app_name.unwrap_or("unknown").to_string(),
            )
        };

    let payload_tenant_id = match payload.tenant_id.as_deref() {
        Some(tenant_id) => match uuid::Uuid::parse_str(tenant_id) {
            Ok(tenant_id) => Some(tenant_id),
            Err(error) => {
                tracing::warn!(
                    %service_id,
                    action_type = %action_type,
                    error = %error,
                    "dropping service audit event with invalid tenant_id"
                );
                return false;
            }
        },
        None => None,
    };

    let target_tenant_id = match scope {
        AuditEventScope::TenantOnly => {
            if is_system {
                tracing::warn!(
                    %service_id,
                    action_type = %action_type,
                    "dropping tenant-scoped audit event from system service"
                );
                return false;
            }
            let tenant_id = match (payload_tenant_id, resolved_tenant_id) {
                (Some(payload_tenant_id), Some(bound_tenant_id))
                    if payload_tenant_id != bound_tenant_id =>
                {
                    tracing::warn!(
                        %service_id,
                        action_type = %action_type,
                        "dropping tenant-scoped audit event with mismatched tenant_id"
                    );
                    return false;
                }
                (Some(payload_tenant_id), _) => payload_tenant_id,
                (None, Some(bound_tenant_id)) => bound_tenant_id,
                (None, None) => {
                    tracing::warn!(
                        %service_id,
                        action_type = %action_type,
                        "dropping tenant-scoped audit event without tenant_id"
                    );
                    return false;
                }
            };
            Some(tenant_id)
        }
        AuditEventScope::ServiceBound => {
            if is_system {
                if payload_tenant_id.is_some() || resolved_tenant_id.is_some() {
                    tracing::warn!(
                        %service_id,
                        action_type = %action_type,
                        "dropping service-bound system audit event with tenant_id"
                    );
                    return false;
                }
                None
            } else {
                let tenant_id = match (payload_tenant_id, resolved_tenant_id) {
                    (Some(payload_tenant_id), Some(bound_tenant_id))
                        if payload_tenant_id != bound_tenant_id =>
                    {
                        tracing::warn!(
                            %service_id,
                            action_type = %action_type,
                            "dropping service-bound audit event with mismatched tenant_id"
                        );
                        return false;
                    }
                    (Some(payload_tenant_id), _) => payload_tenant_id,
                    (None, Some(bound_tenant_id)) => bound_tenant_id,
                    (None, None) => {
                        tracing::warn!(
                            %service_id,
                            action_type = %action_type,
                            "dropping service-bound audit event without tenant_id"
                        );
                        return false;
                    }
                };
                Some(tenant_id)
            }
        }
        AuditEventScope::SystemOnly => {
            if !is_system {
                tracing::warn!(
                    %service_id,
                    action_type = %action_type,
                    "dropping system-scoped audit event from tenant service"
                );
                return false;
            }
            if payload_tenant_id.is_some() || resolved_tenant_id.is_some() {
                tracing::warn!(
                    %service_id,
                    action_type = %action_type,
                    "dropping system-scoped audit event with tenant_id"
                );
                return false;
            }
            None
        }
    };

    let mut builder =
        uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(action_type)
            .actor_service(service_id)
            .actor_display_opt(Some(resolved_service_app_name))
            .target_opt(
                payload.target_type.clone(),
                payload.target_id.clone(),
                payload.target_display.clone(),
            )
            .outcome(outcome)
            .request_id_opt(payload.request_id.clone())
            .correlation_id_opt(payload.correlation_id);
    builder = if let Some(tenant_id) = target_tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };
    if let Some(details_json) = details_json {
        builder = builder.details(details_json);
    }
    let entry = match builder.build() {
        Ok(entry) => entry,
        Err(error) => {
            tracing::warn!(
                %service_id,
                action_type = %payload.action_type,
                error = %error,
                "dropping invalid service audit entry"
            );
            return false;
        }
    };
    state.audit_emitter.emit_event(entry);
    true
}

pub(super) async fn emit_service_enrollment_completed_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
) {
    let payload = AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED.to_string(),
        tenant_id: None,
        target_type: Some("service".to_string()),
        target_id: Some(service_id.to_string()),
        target_display: Some(resolve_service_target_display(state, service_id, false).await),
        outcome: uptrakit_audit_log::AuditOutcome::Success
            .as_str()
            .to_string(),
        details_json: Some(serde_json::json!({ "service_id": service_id }).to_string()),
        request_id: None,
        correlation_id: None,
    };
    let _ = ingest_service_audit_event(state, service_id, false, None, None, payload).await;
}

pub(super) async fn emit_service_certificate_issue_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
    not_after: time::OffsetDateTime,
) {
    let is_system = sys_svc_entity::Entity::find_by_id(service_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .is_some();
    let payload = AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_ISSUE.to_string(),
        tenant_id: None,
        target_type: Some("service".to_string()),
        target_id: Some(service_id.to_string()),
        target_display: Some(resolve_service_target_display(state, service_id, is_system).await),
        outcome: uptrakit_audit_log::AuditOutcome::Success
            .as_str()
            .to_string(),
        details_json: Some(
            serde_json::json!({
                "not_after": not_after.to_string(),
            })
            .to_string(),
        ),
        request_id: None,
        correlation_id: None,
    };
    let _ = ingest_service_audit_event(state, service_id, is_system, None, None, payload).await;
}

pub(super) async fn emit_service_certificate_renew_audit_event(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    is_system: bool,
    not_after: time::OffsetDateTime,
) {
    let payload = AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW.to_string(),
        tenant_id: None,
        target_type: Some("service".to_string()),
        target_id: Some(service_id.to_string()),
        target_display: Some(resolve_service_target_display(state, service_id, is_system).await),
        outcome: uptrakit_audit_log::AuditOutcome::Success
            .as_str()
            .to_string(),
        details_json: Some(
            serde_json::json!({
                "not_after": not_after.to_string(),
            })
            .to_string(),
        ),
        request_id: None,
        correlation_id: None,
    };
    let _ = ingest_service_audit_event(state, service_id, is_system, None, None, payload).await;
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::{MessageProcessor, ProcessorAction};
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;
    use uptrakit_wire::report_tracker::ReportTracker;
    use uptrakit_wire::{AuditEventPayload, ServiceMessage};
    use uuid::Uuid;

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn forwarded_service_audit_event_writes_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type: uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STARTED
                        .to_string(),
                    tenant_id: Some(tenant_id.to_string()),
                    target_type: Some("update_history".to_string()),
                    target_id: Some(Uuid::now_v7().to_string()),
                    target_display: Some("nginx on node-1".to_string()),
                    outcome: uptrakit_audit_log::AuditOutcome::Success
                        .as_str()
                        .to_string(),
                    details_json: Some(serde_json::json!({ "interactive": false }).to_string()),
                    request_id: None,
                    correlation_id: None,
                }),
                None,
            )
            .await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STARTED,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("update_history"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn forwarded_runtime_audit_event_from_tenant_service_writes_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type: uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_GATE
                        .to_string(),
                    tenant_id: Some(tenant_id.to_string()),
                    target_type: None,
                    target_id: None,
                    target_display: None,
                    outcome: uptrakit_audit_log::AuditOutcome::Denied
                        .as_str()
                        .to_string(),
                    details_json: Some(
                        serde_json::json!({
                            "message_name": "ExecuteUpdate",
                            "gate": "freeze",
                        })
                        .to_string(),
                    ),
                    request_id: None,
                    correlation_id: None,
                }),
                None,
            )
            .await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_GATE,
        )
        .await;
        assert_eq!(row.tenant_id, tenant_id);
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn forwarded_scheduler_audit_event_from_system_service_writes_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: true,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-scheduler".to_string()),
            service_tenant_id: None,
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type:
                        uptrakit_audit_log::AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP
                            .to_string(),
                    tenant_id: None,
                    target_type: None,
                    target_id: None,
                    target_display: None,
                    outcome: uptrakit_audit_log::AuditOutcome::Success
                        .as_str()
                        .to_string(),
                    details_json: Some(
                        serde_json::json!({
                            "tenant_deleted": 1,
                            "system_deleted": 2,
                            "retention_days": 90,
                        })
                        .to_string(),
                    ),
                    request_id: None,
                    correlation_id: None,
                }),
                None,
            )
            .await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP,
        )
        .await;
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn local_service_certificate_issue_audit_event_writes_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;

        emit_service_certificate_issue_audit_event(
            &state,
            service_id,
            time::OffsetDateTime::now_utc() + time::Duration::days(30),
        )
        .await;

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_ISSUE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn local_system_service_certificate_issue_audit_event_writes_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;

        emit_service_certificate_issue_audit_event(
            &state,
            service_id,
            time::OffsetDateTime::now_utc() + time::Duration::days(30),
        )
        .await;

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_ISSUE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn local_system_service_certificate_renew_audit_event_writes_system_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_system_service_row(&db, service_id, "uptrakit-scheduler").await;

        emit_service_certificate_renew_audit_event(
            &state,
            service_id,
            true,
            time::OffsetDateTime::now_utc() + time::Duration::days(30),
        )
        .await;

        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn local_service_enrollment_completed_audit_event_writes_tenant_audit_row() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;

        emit_service_enrollment_completed_audit_event(&state, service_id).await;

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_ENROLLMENT_COMPLETED,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(service_id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(service_id.to_string().as_str())
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn invalid_forwarded_service_audit_event_is_dropped_without_disconnect() {
        let surface_registry = Arc::new(crate::surface_registry::SurfaceRegistry::new(
            crate::surface_registry::SurfaceRegistryConfig::default(),
        ));
        let surface_proxy = Arc::new(crate::surface_proxy::SurfaceProxy::new());
        let state = build_handler_test_state(surface_registry, surface_proxy).await;
        let mut processor = MessageProcessor {
            state,
            service_id: Uuid::now_v7(),
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent".to_string()),
            service_tenant_id: Some(Uuid::now_v7()),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type: "auth.login.failed".to_string(),
                    tenant_id: Some(Uuid::now_v7().to_string()),
                    target_type: None,
                    target_id: None,
                    target_display: None,
                    outcome: "validation_failed".to_string(),
                    details_json: None,
                    request_id: None,
                    correlation_id: None,
                }),
                None,
            )
            .await;

        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn forwarded_stateful_audit_event_is_rejected_with_warning() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let service_id = Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent").await;
        let mut processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: false,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };

        let response = processor
            .dispatch(
                ServiceMessage::AuditEvent(AuditEventPayload {
                    action_type: uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE
                        .to_string(),
                    tenant_id: Some(tenant_id.to_string()),
                    target_type: Some("plugin_config".to_string()),
                    target_id: Some(Uuid::now_v7().to_string()),
                    target_display: None,
                    outcome: uptrakit_audit_log::AuditOutcome::Success
                        .as_str()
                        .to_string(),
                    details_json: None,
                    request_id: None,
                    correlation_id: None,
                }),
                None,
            )
            .await;

        // Connection must stay alive
        assert!(response.replies.is_empty());
        assert!(matches!(response.action, ProcessorAction::Continue));

        // No audit row must be written
        let count: u64 = {
            use sea_orm::PaginatorTrait;
            uptrakit_shared_db::entity::audit_log::Entity::find()
                .count(&db)
                .await
                .expect("count audit_log rows")
        };
        assert_eq!(
            count, 0,
            "no audit row should be written for forwarded Stateful event"
        );
    }
}
