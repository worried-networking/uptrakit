//! Common message handlers extracted from the authenticated loop.
//!
//! Each function corresponds to one match arm in the main dispatch and returns
//! a [`LoopAction`] plus an optional [`ControllerMessage`] reply. The main
//! loop is responsible for serializing and writing the reply to the WebSocket
//! sink.

#![expect(
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]

use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use uptrakit_shared_db::entity::service;
use uptrakit_wire::{
    ControllerMessage, ReportPluginConfigPayload, ReportPluginConfigResponsePayload,
};

use super::audit_service::{
    emit_service_certificate_renew_audit_event, ingest_service_audit_event,
};
use super::discovery::trigger_discovery_for_agent_host;
use super::message_processor::LoopAction;
use super::renewal::{sign_renewal_csr, sign_renewal_csr_system};
use super::shared_types::{ProcessorResponse, load_linked_host_ids};
use crate::AppState;

mod certificate;
mod discovery;
mod hosts;
mod shared;
mod version_check;

pub(super) use certificate::handle_renew_certificate;
pub(super) use discovery::handle_discovery_results;
pub(super) use hosts::handle_report_hosts;
use shared::emit_service_inventory_audit;
pub(super) use shared::handle_ping;
pub(super) use version_check::handle_version_check_results;

// ---------------------------------------------------------------------------
// handle_report_plugin_config
// ---------------------------------------------------------------------------

fn report_plugin_config_target_id(plugin_type: &str, config_name: &str) -> String {
    format!("service_reported:{plugin_type}:{config_name}")
}

struct PluginConfigReportAuditCtx<'a> {
    state: &'a AppState,
    service_id: uuid::Uuid,
    service_tenant_id: Option<uuid::Uuid>,
    service_app_name: Option<&'a str>,
}

fn emit_report_plugin_config_audit(
    ctx: &PluginConfigReportAuditCtx<'_>,
    request_id: &str,
    plugin_type: &str,
    config_name: &str,
    target_id: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let mut details = serde_json::json!({
        "plugin_type": plugin_type,
        "config_name": config_name,
        "mutation_source": "service_ws.report_plugin_config",
    });
    if let Some(service_app_name) = ctx.service_app_name {
        details["service_app_name"] = serde_json::Value::String(service_app_name.to_string());
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::Value::String(reason_code.to_string());
    }

    let mut builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .actor_service(ctx.service_id)
    .actor_display_opt(ctx.service_app_name.map(str::to_string))
    .target(
        "plugin_config",
        target_id.unwrap_or_else(|| report_plugin_config_target_id(plugin_type, config_name)),
        Some(config_name.to_string()),
    )
    .outcome(outcome)
    .details(details)
    .request_id_opt(Some(request_id.to_string()));
    builder = if let Some(tenant_id) = ctx.service_tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };

    match builder.build() {
        Ok(entry) => ctx.state.audit_emitter.emit_event(entry),
        Err(error) => tracing::warn!(
            error = %error,
            service_id = %ctx.service_id,
            plugin_type,
            config_name,
            outcome = outcome.as_str(),
            "failed to build ReportPluginConfig audit entry"
        ),
    }
}

/// After an `AwaitingRestart` record transitions to `Completed` or `Failed`,
/// emit a per-item `BatchProgressEvent`, then promote the next queued update
/// for the same host (batch or standalone).  If the batch is now complete,
/// `handle_batch_completion` is called to emit the final summary and send
/// batch-completion notifications.
async fn trigger_host_progression_after_awaiting_restart(
    state: &Arc<AppState>,
    hsi_id: uuid::Uuid,
) {
    use sea_orm::QueryOrder;
    use uptrakit_shared_db::entity::update_history;

    // Load the record just transitioned from AwaitingRestart.
    // Filter on awaiting_restart_since IS NOT NULL to avoid picking up
    // unrelated records that happened to end up Completed/Failed.
    let record = match update_history::Entity::find()
        .filter(update_history::Column::HostSoftwareItemId.eq(hsi_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Completed,
            update_history::UpdateStatus::Failed,
        ]))
        .filter(update_history::Column::AwaitingRestartSince.is_not_null())
        .order_by_desc(update_history::Column::CompletedAt)
        .one(state.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(
                host_software_item_id = %hsi_id,
                "no Completed/Failed record found after AwaitingRestart transition"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                host_software_item_id = %hsi_id,
                "failed to load update_history for post-AwaitingRestart dispatch"
            );
            return;
        }
    };

    let dispatch = crate::queries::update_dispatch::DispatchContext {
        notifier: &state.notification.notification_service,
        protection: state.controller_update_protection(),
        #[cfg(feature = "plugin-ops")]
        hook: state.controller_update_hook(),
        #[cfg(feature = "plugin-ops")]
        notification_ops: Some(state.plugin.plugin_ops.as_ref()),
    };

    if let Some(batch_id) = record.batch_id {
        // Emit per-item progress event before dispatching next — mirrors
        // what handle_update_result does in updates.rs.
        let event = match record.status {
            update_history::UpdateStatus::Completed => {
                crate::batch_progress_broadcaster::BatchProgressEvent::UpdateCompleted {
                    update_history_id: record.id,
                    software_item_name: super::updates::resolve_software_item_name(
                        state,
                        record.software_item_id,
                    )
                    .await,
                    host_name: super::updates::resolve_host_name(state, record.host_id).await,
                }
            }
            _ => crate::batch_progress_broadcaster::BatchProgressEvent::UpdateFailed {
                update_history_id: record.id,
                software_item_name: super::updates::resolve_software_item_name(
                    state,
                    record.software_item_id,
                )
                .await,
                host_name: super::updates::resolve_host_name(state, record.host_id).await,
                // The error detail is not stored on the AwaitingRestart record itself.
                error: None,
            },
        };
        super::updates::emit_batch_progress_event(state, batch_id, event).await;

        match crate::queries::update_batches::dispatch_next_in_batch(
            state.db(),
            dispatch,
            batch_id,
            record.host_id,
            record.tenant_id,
        )
        .await
        {
            Ok(Some(completion)) => {
                super::updates::handle_batch_completion(state, batch_id, &completion).await;
            }
            Ok(None) => {
                // Batch still in progress — emit updated progress summary.
                super::updates::emit_batch_progress_from_db(state, batch_id).await;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %batch_id,
                    host_id = %record.host_id,
                    "post-AwaitingRestart batch dispatch failed"
                );
            }
        }
    } else if let Err(e) = crate::queries::update_batches::dispatch_next_queued_for_host(
        state.db(),
        dispatch,
        record.host_id,
        record.tenant_id,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            host_id = %record.host_id,
            "post-AwaitingRestart standalone dispatch failed"
        );
    }
}

// ---------------------------------------------------------------------------
// handle_report_plugin_config
// ---------------------------------------------------------------------------

/// Handle a `ReportPluginConfig` message: find or create a plugin config and
/// return the response message.
///
/// Idempotent: if a config with the same `(tenant_id, plugin_type, name)`
/// already exists, the existing ID is returned without creating a duplicate.
pub(super) async fn handle_report_plugin_config(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &ReportPluginConfigPayload,
) -> ProcessorResponse {
    let request_id = payload.request_id.clone();

    let service_model = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(service_model)) => service_model,
        Ok(None) => {
            tracing::warn!(%service_id, "ReportPluginConfig: service not found");
            emit_report_plugin_config_audit(
                &PluginConfigReportAuditCtx {
                    state,
                    service_id,
                    service_tenant_id: None,
                    service_app_name: None,
                },
                &request_id,
                &payload.plugin_type,
                &payload.name,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("service_not_found"),
            );
            return ProcessorResponse::cont();
        }
        Err(e) => {
            tracing::warn!(%service_id, error = %e, "ReportPluginConfig: DB error");
            emit_report_plugin_config_audit(
                &PluginConfigReportAuditCtx {
                    state,
                    service_id,
                    service_tenant_id: None,
                    service_app_name: None,
                },
                &request_id,
                &payload.plugin_type,
                &payload.name,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("service_lookup_failed"),
            );
            return ProcessorResponse::cont();
        }
    };

    // Validate the plugin type is known
    let plugin_type_id = uptrakit_shared_types::PluginTypeId::new(&payload.plugin_type);
    if let Err(e) = state
        .plugin
        .plugin_ops
        .validate_config(&plugin_type_id, &payload.config)
    {
        tracing::warn!(
            %service_id,
            plugin_type = %payload.plugin_type,
            error = %e,
            "ReportPluginConfig: invalid config"
        );
        emit_report_plugin_config_audit(
            &PluginConfigReportAuditCtx {
                state,
                service_id,
                service_tenant_id: Some(service_model.tenant_id),
                service_app_name: service_model.service_app_name.as_deref(),
            },
            &request_id,
            &payload.plugin_type,
            &payload.name,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            Some("invalid_plugin_config"),
        );
        let resp_payload: ReportPluginConfigResponsePayload =
            serde_json::from_value(serde_json::json!({
                "request_id": request_id,
                "success": false,
                "error": format!("invalid plugin config: {e}"),
            }))
            .expect("ReportPluginConfigResponsePayload JSON is always valid");
        return ProcessorResponse::reply(ControllerMessage::ReportPluginConfigResponse(
            resp_payload,
        ));
    }

    // Find or create the plugin config
    let result = crate::queries::autodiscovery::find_or_create_default_plugin_config(
        state.db(),
        service_model.tenant_id,
        &payload.plugin_type,
        &payload.config,
        &payload.name,
    )
    .await;

    let resp = match result {
        Ok(config_id) => {
            tracing::info!(
                %service_id,
                %config_id,
                plugin_type = %payload.plugin_type,
                name = %payload.name,
                "ReportPluginConfig: config created/found"
            );
            emit_report_plugin_config_audit(
                &PluginConfigReportAuditCtx {
                    state,
                    service_id,
                    service_tenant_id: Some(service_model.tenant_id),
                    service_app_name: service_model.service_app_name.as_deref(),
                },
                &request_id,
                &payload.plugin_type,
                &payload.name,
                Some(config_id.to_string()),
                uptrakit_audit_log::AuditOutcome::Success,
                None,
            );
            let resp_payload: ReportPluginConfigResponsePayload =
                serde_json::from_value(serde_json::json!({
                    "request_id": request_id,
                    "success": true,
                    "plugin_config_id": config_id,
                }))
                .expect("ReportPluginConfigResponsePayload JSON is always valid");
            ControllerMessage::ReportPluginConfigResponse(resp_payload)
        }
        Err(e) => {
            tracing::warn!(
                %service_id,
                error = %e,
                "ReportPluginConfig: failed to create/find config"
            );
            emit_report_plugin_config_audit(
                &PluginConfigReportAuditCtx {
                    state,
                    service_id,
                    service_tenant_id: Some(service_model.tenant_id),
                    service_app_name: service_model.service_app_name.as_deref(),
                },
                &request_id,
                &payload.plugin_type,
                &payload.name,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("create_or_find_failed"),
            );
            let resp_payload: ReportPluginConfigResponsePayload =
                serde_json::from_value(serde_json::json!({
                    "request_id": request_id,
                    "success": false,
                    "error": format!("failed to create plugin config: {e}"),
                }))
                .expect("ReportPluginConfigResponsePayload JSON is always valid");
            ControllerMessage::ReportPluginConfigResponse(resp_payload)
        }
    };

    ProcessorResponse::reply(resp)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod tests;
