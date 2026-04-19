//! Update-tracking message handlers.
//!
//! Handles `ServiceTriggerUpdate` and `ServiceTriggerHostBatchUpdate` messages
//! from services with the `UpdateTracking` capability.

use std::sync::Arc;

use uptrakit_internal_wire::{
    ControllerMessage, ErrorCode, ErrorPayload, ServiceHostBatchUpdateTriggerPayload,
    ServiceUpdateTriggerPayload,
};
use uptrakit_web_api_types::events::AdminEvent;

use super::shared_types::ProcessorResponse;
use crate::AppState;

fn emit_software_update_audit(
    state: &AppState,
    payload: &ServiceUpdateTriggerPayload,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let entry = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
    )
    .tenant_scope(payload.tenant_id)
    .actor_service(payload.actor_service_id)
    .target("software_item", payload.software_item_id.to_string(), None)
    .outcome(outcome)
    .details(details)
    .build();

    match entry {
        Ok(entry) => state.audit_emitter.emit_best_effort(entry),
        Err(error) => {
            tracing::warn!(
                error = %error,
                software_item_id = %payload.software_item_id,
                host_id = %payload.host_id,
                "failed to build service-triggered software update audit entry"
            );
        }
    }
}

fn emit_host_batch_update_audit(
    state: &AppState,
    payload: &ServiceHostBatchUpdateTriggerPayload,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let entry = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_TRIGGERED,
    )
    .tenant_scope(payload.tenant_id)
    .actor_service(payload.actor_service_id)
    .target("host", payload.host_id.to_string(), None)
    .outcome(outcome)
    .details(details)
    .build();

    match entry {
        Ok(entry) => state.audit_emitter.emit_best_effort(entry),
        Err(error) => {
            tracing::warn!(
                error = %error,
                host_id = %payload.host_id,
                "failed to build service-triggered host batch update audit entry"
            );
        }
    }
}

fn classify_trigger_update_audit_failure(
    err: &rootcause::Report<uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError>,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    use uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError;

    match err.current_context() {
        TriggerUpdateError::SoftwareItemNotFound => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "trigger_update.software_item_not_found",
        ),
        TriggerUpdateError::HostNotFound => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "trigger_update.host_not_found",
        ),
        TriggerUpdateError::UpdateAlreadyActive => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "trigger_update.update_already_active",
        ),
        TriggerUpdateError::HostNotAssigned => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_update.host_not_assigned",
        ),
        TriggerUpdateError::NoExecuteUpdatePlugin => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_update.no_execute_update_plugin",
        ),
        TriggerUpdateError::NoAgent => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_update.no_agent",
        ),
        TriggerUpdateError::AgentNotApproved => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_update.agent_not_approved",
        ),
        TriggerUpdateError::PluginConfigNotFound => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_update.plugin_config_not_found",
        ),
        TriggerUpdateError::UnknownPluginType(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_update.unknown_plugin_type",
        ),
        TriggerUpdateError::PreUpdateProtection(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_update.pre_update_protection_failed",
        ),
        TriggerUpdateError::Database(_) => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "trigger_update.database_error",
        ),
        TriggerUpdateError::PostUpdateFinalization(_)
        | TriggerUpdateError::PostUpdateFinalizationTimeout => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "trigger_update.post_update_finalization_failed",
        ),
    }
}

fn classify_trigger_update_dispatch_audit_outcome(
    status: uptrakit_shared_db::entity::update_history::UpdateStatus,
) -> uptrakit_audit_log::AuditOutcome {
    match status {
        uptrakit_shared_db::entity::update_history::UpdateStatus::Failed => {
            uptrakit_audit_log::AuditOutcome::Failed
        }
        _ => uptrakit_audit_log::AuditOutcome::Success,
    }
}

fn trigger_update_dispatch_status_label(
    status: uptrakit_shared_db::entity::update_history::UpdateStatus,
) -> &'static str {
    match status {
        uptrakit_shared_db::entity::update_history::UpdateStatus::Pending => "pending",
        uptrakit_shared_db::entity::update_history::UpdateStatus::Failed => "failed",
        _ => "queued",
    }
}

fn classify_batch_trigger_audit_failure(
    err: &rootcause::Report<uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError>,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    use uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError;

    match err.current_context() {
        TriggerUpdateError::SoftwareItemNotFound => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "trigger_batch_update.software_item_not_found",
        ),
        TriggerUpdateError::HostNotFound => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "trigger_batch_update.host_not_found",
        ),
        TriggerUpdateError::UpdateAlreadyActive => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "trigger_batch_update.update_already_active",
        ),
        TriggerUpdateError::HostNotAssigned => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_batch_update.host_not_assigned",
        ),
        TriggerUpdateError::NoExecuteUpdatePlugin => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_batch_update.no_execute_update_plugin",
        ),
        TriggerUpdateError::NoAgent => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_batch_update.no_agent",
        ),
        TriggerUpdateError::AgentNotApproved => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_batch_update.agent_not_approved",
        ),
        TriggerUpdateError::PluginConfigNotFound => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_batch_update.plugin_config_not_found",
        ),
        TriggerUpdateError::UnknownPluginType(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_batch_update.unknown_plugin_type",
        ),
        TriggerUpdateError::PreUpdateProtection(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "trigger_batch_update.pre_update_protection_failed",
        ),
        TriggerUpdateError::Database(_) => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "trigger_batch_update.database_error",
        ),
        TriggerUpdateError::PostUpdateFinalization(_)
        | TriggerUpdateError::PostUpdateFinalizationTimeout => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "trigger_batch_update.post_update_finalization_failed",
        ),
    }
}

fn batch_trigger_outcome(
    total_created: usize,
    skipped_count: usize,
) -> uptrakit_audit_log::AuditOutcome {
    if total_created == 0 {
        uptrakit_audit_log::AuditOutcome::Failed
    } else if skipped_count == 0 {
        uptrakit_audit_log::AuditOutcome::Success
    } else {
        uptrakit_audit_log::AuditOutcome::Partial
    }
}

/// Handle a `ServiceTriggerUpdate` message.
#[tracing::instrument(skip_all)]
pub(super) async fn handle_service_trigger_update(
    state: &Arc<AppState>,
    service_app_name: &str,
    payload: &ServiceUpdateTriggerPayload,
) -> ProcessorResponse {
    match crate::queries::update_triggers::trigger_update_for_host(
        state.db(),
        crate::queries::update_dispatch::DispatchContext {
            notifier: &state.notification.notification_service,
            protection: state.controller_update_protection(),
        },
        crate::queries::update_triggers::TriggerUpdateParams {
            tenant_id: payload.tenant_id,
            item_id: payload.software_item_id,
            host_id: payload.host_id,
            to_version: payload.to_version.clone(),
            actor_type: service_app_name,
            actor_id: &payload.actor_service_id.to_string(),
            release_info: None,
            interactive: false,
        },
    )
    .await
    {
        Ok(result) => {
            let dispatch_status = trigger_update_dispatch_status_label(result.initial_status);
            emit_software_update_audit(
                state,
                payload,
                classify_trigger_update_dispatch_audit_outcome(result.initial_status),
                serde_json::json!({
                    "host_id": payload.host_id,
                    "to_version": payload.to_version,
                    "interactive": false,
                    "update_history_id": result.update_history_id,
                    "dispatch_status": dispatch_status,
                }),
            );
            tracing::info!(
                update_id = %result.update_history_id,
                software_item_id = %payload.software_item_id,
                host_id = %payload.host_id,
                "service-triggered update dispatched"
            );
            state
                .notification
                .notification_service
                .push_software_states_for_tenant(state.db(), payload.tenant_id)
                .await;
            state
                .notification
                .event_broadcaster
                .send(
                    payload.tenant_id,
                    AdminEvent::UpdateTriggered {
                        update_history_id: result.update_history_id,
                        host_id: payload.host_id,
                        software_item_id: payload.software_item_id,
                    },
                )
                .await;
            ProcessorResponse::cont()
        }
        Err(err) => {
            let (outcome, reason_code) = classify_trigger_update_audit_failure(&err);
            emit_software_update_audit(
                state,
                payload,
                outcome,
                serde_json::json!({
                    "host_id": payload.host_id,
                    "to_version": payload.to_version,
                    "interactive": false,
                    "reason_code": reason_code,
                }),
            );
            tracing::warn!(
                error = %err,
                software_item_id = %payload.software_item_id,
                host_id = %payload.host_id,
                "service-triggered update failed"
            );
            ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: err.to_string(),
            }))
        }
    }
}

/// Handle a `ServiceTriggerHostBatchUpdate` message.
#[tracing::instrument(skip_all)]
pub(super) async fn handle_service_trigger_host_batch_update(
    state: &Arc<AppState>,
    service_app_name: &str,
    payload: &ServiceHostBatchUpdateTriggerPayload,
) -> ProcessorResponse {
    let category_filter = if payload.security_only {
        Some("security")
    } else {
        None
    };
    let outdated = match crate::queries::update_batches::find_outdated_items_for_host(
        state.db(),
        payload.tenant_id,
        payload.host_id,
        category_filter,
        None,
    )
    .await
    {
        Ok(items) => items,
        Err(err) => {
            let (outcome, reason_code) = classify_batch_trigger_audit_failure(&err);
            emit_host_batch_update_audit(
                state,
                payload,
                outcome,
                serde_json::json!({
                    "batch_scope": "host",
                    "category_filter_present": payload.security_only,
                    "excluded_item_count": 0,
                    "reason_code": reason_code,
                }),
            );
            tracing::warn!(
                error = %err,
                host_id = %payload.host_id,
                "service-triggered host batch update: failed to find outdated items"
            );
            return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: err.to_string(),
            }));
        }
    };

    if outdated.is_empty() {
        emit_host_batch_update_audit(
            state,
            payload,
            uptrakit_audit_log::AuditOutcome::Failed,
            serde_json::json!({
                "batch_scope": "host",
                "category_filter_present": payload.security_only,
                "excluded_item_count": 0,
                "reason_code": "trigger_batch_update.no_outdated_items",
            }),
        );
        return ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::BadRequest,
            message: "no outdated items found on this host".to_string(),
        }));
    }

    let params = crate::queries::update_batches::CreateBatchParams {
        tenant_id: payload.tenant_id,
        batch_type: crate::queries::update_types::BatchType::HostUpdate,
        actor_type: service_app_name,
        actor_id: &payload.actor_service_id.to_string(),
    };
    match crate::queries::update_batches::create_batch(
        state.db(),
        crate::queries::update_dispatch::DispatchContext {
            notifier: &state.notification.notification_service,
            protection: state.controller_update_protection(),
        },
        &params,
        outdated,
    )
    .await
    {
        Ok(resp) => {
            let skipped_count = resp.skipped.len();
            let audit_outcome = batch_trigger_outcome(resp.total_created, skipped_count);
            emit_host_batch_update_audit(
                state,
                payload,
                audit_outcome,
                serde_json::json!({
                    "batch_scope": "host",
                    "batch_id": resp.batch_id,
                    "accepted_count": resp.total_created,
                    "skipped_count": skipped_count,
                    "category_filter_present": payload.security_only,
                    "excluded_item_count": 0,
                    "no_op": resp.total_created == 0,
                }),
            );
            if let Some(batch_id) = resp.batch_id {
                tracing::info!(
                    %batch_id,
                    host_id = %payload.host_id,
                    "service-triggered host batch update dispatched"
                );
                state
                    .notification
                    .notification_service
                    .push_software_states_for_tenant(state.db(), payload.tenant_id)
                    .await;
                ProcessorResponse::cont()
            } else {
                ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::BadRequest,
                    message: "no eligible items for batch update".to_string(),
                }))
            }
        }
        Err(err) => {
            let (outcome, reason_code) = classify_batch_trigger_audit_failure(&err);
            emit_host_batch_update_audit(
                state,
                payload,
                outcome,
                serde_json::json!({
                    "batch_scope": "host",
                    "category_filter_present": payload.security_only,
                    "excluded_item_count": 0,
                    "reason_code": reason_code,
                }),
            );
            tracing::warn!(
                error = %err,
                host_id = %payload.host_id,
                "service-triggered host batch update failed"
            );
            ProcessorResponse::reply(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::BadRequest,
                message: err.to_string(),
            }))
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
    use std::{future::Future, pin::Pin, sync::Arc};
    use time::OffsetDateTime;
    use uptrakit_plugin_infrastructure_registry::{
        CatalogConfig, ControllerPostUpdateContext, ControllerProtectionContext,
        ControllerProtectionDecision, ControllerUpdateProtection, ControllerUpdateProtectionOps,
        NotificationOps, NotificationTransport, PluginConfigOps, PluginMetadataOps, PluginOps,
        PluginResult, PluginSurfaceActionOps, PluginSurfaceOps, PostUpdateOutcome,
        SoftwareItemCreatedEvent, SoftwareItemLifecycle, SoftwareItemLifecycleContext,
        SoftwareItemLifecycleOps, SoftwareItemPatch, SurfaceActionContext, build_catalog,
    };
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, plugin_config, service, service_host,
        software_item,
    };
    use uptrakit_shared_types::{PluginTypeId, ServiceStatus};
    use uuid::Uuid;

    struct AlwaysSkipProtection;

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for AlwaysSkipProtection {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::new("infra_test_always_skip_protection")
        }
    }

    #[async_trait]
    impl ControllerUpdateProtection for AlwaysSkipProtection {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> PluginResult<ControllerProtectionDecision> {
            Ok(ControllerProtectionDecision::skipped(None))
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> PluginResult<PostUpdateOutcome> {
            Ok(PostUpdateOutcome::default())
        }
    }

    struct ProtectionOverridePluginOps {
        inner: Arc<dyn PluginOps>,
        protection: Arc<dyn ControllerUpdateProtection>,
    }

    impl PluginMetadataOps for ProtectionOverridePluginOps {
        fn get(
            &self,
            id: &uptrakit_shared_types::PluginTypeId,
        ) -> Option<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
            self.inner.get(id)
        }

        fn all(&self) -> Vec<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
            self.inner.all()
        }
    }

    impl PluginConfigOps for ProtectionOverridePluginOps {}

    impl PluginSurfaceActionOps for ProtectionOverridePluginOps {
        fn handle_surface_action<'a>(
            &'a self,
            ctx: &'a SurfaceActionContext<'a>,
            surface_id: &'a str,
            action_id: &'a str,
            params: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>>
        {
            self.inner
                .handle_surface_action(ctx, surface_id, action_id, params)
        }
    }

    impl PluginSurfaceOps for ProtectionOverridePluginOps {
        fn surface_registrations(
            &self,
        ) -> Vec<uptrakit_internal_wire::surfaces::SurfaceRegistration> {
            self.inner.surface_registrations()
        }
    }

    impl NotificationOps for ProtectionOverridePluginOps {
        fn transport(
            &self,
            id: &uptrakit_shared_types::PluginTypeId,
        ) -> Option<Arc<dyn NotificationTransport>> {
            self.inner.transport(id)
        }

        fn notification_supported_types(&self) -> Vec<uptrakit_shared_types::PluginTypeId> {
            self.inner.notification_supported_types()
        }
    }

    impl SoftwareItemLifecycleOps for ProtectionOverridePluginOps {
        fn on_software_item_created<'a>(
            &'a self,
            event: &'a SoftwareItemCreatedEvent,
            ctx: &'a SoftwareItemLifecycleContext,
        ) -> Pin<Box<dyn Future<Output = Option<SoftwareItemPatch>> + Send + 'a>> {
            self.inner.on_software_item_created(event, ctx)
        }

        fn software_item_lifecycle_plugins(&self) -> &[Arc<dyn SoftwareItemLifecycle>] {
            self.inner.software_item_lifecycle_plugins()
        }
    }

    impl ControllerUpdateProtectionOps for ProtectionOverridePluginOps {
        fn controller_update_protection(&self) -> Option<Arc<dyn ControllerUpdateProtection>> {
            Some(self.protection.clone())
        }
    }

    async fn build_test_state_with_protection(
        db: sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        protection: Arc<dyn ControllerUpdateProtection>,
    ) -> Arc<AppState> {
        let base_plugin_ops: Arc<dyn PluginOps> = Arc::new(
            build_catalog(&CatalogConfig::default()).expect("catalog should build in tests"),
        );
        let plugin_ops: Arc<dyn PluginOps> = Arc::new(ProtectionOverridePluginOps {
            inner: base_plugin_ops,
            protection,
        });
        let (state, _jwt) =
            crate::test_harness::build_test_state_with_plugin_ops(db, tenant_id, Some(plugin_ops))
                .await;
        state
    }

    async fn insert_service_row(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        service_id: Uuid,
    ) {
        let now = OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("svc-{service_id}")),
            friendly_name: Set(format!("Service {service_id}")),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(Some("uptrakit-mqtt".to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert service");
    }

    async fn insert_host_row(db: &sea_orm::DatabaseConnection, tenant_id: Uuid) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let host_id = Uuid::now_v7();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{host_id}")),
            hostname: Set(format!("host-{host_id}")),
            friendly_name: Set(format!("Host {host_id}")),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host")
        .id
    }

    async fn insert_service_host_link(
        db: &sea_orm::DatabaseConnection,
        service_id: Uuid,
        host_id: Uuid,
    ) {
        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(db)
        .await
        .expect("insert service-host link");
    }

    async fn insert_software_item(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        name: &str,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert software item")
        .id
    }

    async fn insert_plugin_config(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        plugin_config_id: Uuid,
        active: bool,
    ) {
        let now = OffsetDateTime::now_utc();
        plugin_config::ActiveModel {
            id: Set(plugin_config_id),
            tenant_id: Set(tenant_id),
            name: Set(format!("cfg-{plugin_config_id}")),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set((!active).then_some(now)),
        }
        .insert(db)
        .await
        .expect("insert plugin config");
    }

    async fn insert_host_software_item(
        db: &sea_orm::DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Option<Uuid>,
        installed_version: &str,
        latest_version: &str,
        update_category: &str,
    ) -> Uuid {
        let hsi_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(hsi_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(None),
            plugin_config_id: Set(plugin_config_id),
            package_identifier: Set(Some("org/repo".to_string())),
            installed_version: Set(Some(installed_version.to_string())),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(Some(latest_version.to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(OffsetDateTime::now_utc()),
            update_category: Set(update_category.to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host software item")
        .id
    }

    async fn insert_execute_update_assignment(
        db: &sea_orm::DatabaseConnection,
        host_id: Uuid,
        software_item_id: Uuid,
        host_software_item_id: Uuid,
        plugin_config_id: Option<Uuid>,
    ) {
        let now = OffsetDateTime::now_utc();
        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(plugin_config_id),
            plugin_type: Set("releases_github".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert execute_update assignment");
    }

    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: &'static str,
    ) -> uptrakit_shared_db::entity::audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = uptrakit_shared_db::entity::audit_log::Entity::find()
                .filter(uptrakit_shared_db::entity::audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(uptrakit_shared_db::entity::audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    struct UpdateFixture {
        service_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
    }

    async fn insert_update_fixture(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
    ) -> UpdateFixture {
        let service_id = Uuid::now_v7();
        insert_service_row(db, tenant_id, service_id).await;
        let host_id = insert_host_row(db, tenant_id).await;
        insert_service_host_link(db, service_id, host_id).await;

        let software_item_id = insert_software_item(db, tenant_id, "nginx").await;
        let valid_plugin_config_id = Uuid::now_v7();
        insert_plugin_config(db, tenant_id, valid_plugin_config_id, true).await;
        let hsi_id = insert_host_software_item(
            db,
            host_id,
            software_item_id,
            Some(valid_plugin_config_id),
            "1.0.0",
            "1.1.0",
            "security",
        )
        .await;
        insert_execute_update_assignment(
            db,
            host_id,
            software_item_id,
            hsi_id,
            Some(valid_plugin_config_id),
        )
        .await;

        UpdateFixture {
            service_id,
            host_id,
            software_item_id,
        }
    }

    #[tokio::test]
    async fn service_trigger_update_writes_success_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state =
            build_test_state_with_protection(db.clone(), tenant_id, Arc::new(AlwaysSkipProtection))
                .await;
        let fixture = insert_update_fixture(&db, tenant_id).await;
        let payload = ServiceUpdateTriggerPayload {
            tenant_id,
            software_item_id: fixture.software_item_id,
            host_id: fixture.host_id,
            to_version: "1.1.0".to_string(),
            actor_service_id: fixture.service_id,
        };

        let response = handle_service_trigger_update(&state, "uptrakit-mqtt", &payload).await;
        assert!(
            response.replies.is_empty(),
            "unexpected replies: {:?}",
            response.replies
        );
        assert!(matches!(
            response.action,
            super::super::shared_types::ProcessorAction::Continue
        ));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(fixture.service_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(fixture.software_item_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["host_id"], serde_json::json!(fixture.host_id));
        assert_eq!(details["to_version"], serde_json::json!("1.1.0"));
        assert_eq!(details["interactive"], serde_json::json!(false));
        assert_eq!(details["dispatch_status"], serde_json::json!("pending"));
    }

    #[tokio::test]
    async fn service_trigger_update_host_not_assigned_writes_validation_failed_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state =
            build_test_state_with_protection(db.clone(), tenant_id, Arc::new(AlwaysSkipProtection))
                .await;
        let fixture = insert_update_fixture(&db, tenant_id).await;
        let unassigned_host_id = insert_host_row(&db, tenant_id).await;
        insert_service_host_link(&db, fixture.service_id, unassigned_host_id).await;
        let payload = ServiceUpdateTriggerPayload {
            tenant_id,
            software_item_id: fixture.software_item_id,
            host_id: unassigned_host_id,
            to_version: "1.1.0".to_string(),
            actor_service_id: fixture.service_id,
        };

        let response = handle_service_trigger_update(&state, "uptrakit-mqtt", &payload).await;
        assert_eq!(response.replies.len(), 1);
        assert!(matches!(
            response.action,
            super::super::shared_types::ProcessorAction::Continue
        ));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("trigger_update.host_not_assigned")
        );
    }

    #[tokio::test]
    async fn service_trigger_update_missing_item_writes_denied_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state =
            build_test_state_with_protection(db.clone(), tenant_id, Arc::new(AlwaysSkipProtection))
                .await;
        let fixture = insert_update_fixture(&db, tenant_id).await;
        let missing_item_id = Uuid::now_v7();
        let payload = ServiceUpdateTriggerPayload {
            tenant_id,
            software_item_id: missing_item_id,
            host_id: fixture.host_id,
            to_version: "1.1.0".to_string(),
            actor_service_id: fixture.service_id,
        };

        let response = handle_service_trigger_update(&state, "uptrakit-mqtt", &payload).await;
        assert_eq!(response.replies.len(), 1);
        assert!(matches!(
            response.action,
            super::super::shared_types::ProcessorAction::Continue
        ));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_item_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("trigger_update.software_item_not_found")
        );
    }

    #[tokio::test]
    async fn service_trigger_host_batch_update_writes_partial_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state =
            build_test_state_with_protection(db.clone(), tenant_id, Arc::new(AlwaysSkipProtection))
                .await;
        let fixture = insert_update_fixture(&db, tenant_id).await;

        let item_skip = insert_software_item(&db, tenant_id, "redis").await;
        let missing_plugin_config_id = Uuid::now_v7();
        insert_plugin_config(&db, tenant_id, missing_plugin_config_id, false).await;
        let hsi_skip = insert_host_software_item(
            &db,
            fixture.host_id,
            item_skip,
            Some(missing_plugin_config_id),
            "7.0.0",
            "7.1.0",
            "feature",
        )
        .await;
        insert_execute_update_assignment(
            &db,
            fixture.host_id,
            item_skip,
            hsi_skip,
            Some(missing_plugin_config_id),
        )
        .await;

        let payload = ServiceHostBatchUpdateTriggerPayload {
            tenant_id,
            host_id: fixture.host_id,
            actor_service_id: fixture.service_id,
            security_only: false,
        };

        let response =
            handle_service_trigger_host_batch_update(&state, "uptrakit-mqtt", &payload).await;
        assert!(response.replies.is_empty());
        assert!(matches!(
            response.action,
            super::super::shared_types::ProcessorAction::Continue
        ));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Partial.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("host"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(fixture.host_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["batch_scope"], serde_json::json!("host"));
        assert_eq!(details["accepted_count"], serde_json::json!(1));
        assert_eq!(details["skipped_count"], serde_json::json!(1));
        assert_eq!(details["category_filter_present"], serde_json::json!(false));
        assert_eq!(details["excluded_item_count"], serde_json::json!(0));
        assert_eq!(details["no_op"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn service_trigger_host_batch_update_no_outdated_writes_failed_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state =
            build_test_state_with_protection(db.clone(), tenant_id, Arc::new(AlwaysSkipProtection))
                .await;
        let service_id = Uuid::now_v7();
        insert_service_row(&db, tenant_id, service_id).await;
        let host_id = insert_host_row(&db, tenant_id).await;
        insert_service_host_link(&db, service_id, host_id).await;
        let payload = ServiceHostBatchUpdateTriggerPayload {
            tenant_id,
            host_id,
            actor_service_id: service_id,
            security_only: false,
        };

        let response =
            handle_service_trigger_host_batch_update(&state, "uptrakit-mqtt", &payload).await;
        assert_eq!(response.replies.len(), 1);
        assert!(matches!(
            response.action,
            super::super::shared_types::ProcessorAction::Continue
        ));

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("trigger_batch_update.no_outdated_items")
        );
    }
}
