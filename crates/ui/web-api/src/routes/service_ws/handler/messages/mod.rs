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

use uptrakit_shared_db::entity::{host, service, service_host};
use uptrakit_wire::report_tracker::{PageOutcome, ReportTracker};
use uptrakit_wire::{
    ControllerMessage, DiscoveryResultsPayload, ReportPagination, ReportPluginConfigPayload,
    ReportPluginConfigResponsePayload,
};

use super::audit_service::{
    emit_service_certificate_renew_audit_event, ingest_service_audit_event,
};
use super::discovery::trigger_discovery_for_agent_host;
use super::message_processor::LoopAction;
use super::renewal::{sign_renewal_csr, sign_renewal_csr_system};
use super::shared_types::{ProcessorResponse, load_linked_host_ids};
use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};

mod certificate;
mod hosts;
mod shared;
mod version_check;

pub(super) use certificate::handle_renew_certificate;
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
// handle_discovery_results
// ---------------------------------------------------------------------------

/// Find the host linked to a service that matches the given `machine_id`.
///
/// Iterates the provided service-host links and queries the DB for each
/// until a matching, non-deactivated host is found.
async fn find_linked_host_by_machine_id(
    db: &sea_orm::DatabaseConnection,
    links: &[service_host::Model],
    machine_id: &str,
) -> Option<uuid::Uuid> {
    for link in links {
        if let Ok(Some(h)) = host::Entity::find_by_id(link.host_id)
            .filter(host::Column::MachineId.eq(machine_id))
            .filter(host::Column::DeactivatedAt.is_null())
            .one(db)
            .await
        {
            return Some(h.id);
        }
    }
    None
}

/// Process a single discovery page for a known host.
///
/// Calls [`process_discovery_results`] and, on the final page, dispatches
/// a [`NotificationEventDetails::NewSoftwareDiscovered`] notification when
/// at least one item was found.
async fn process_discovery_page_for_host(
    state: &Arc<AppState>,
    svc: &service::Model,
    host_id: uuid::Uuid,
    payload: DiscoveryResultsPayload,
    page_outcome: PageOutcome,
    pagination: Option<&ReportPagination>,
    report_tracker: &mut ReportTracker,
) -> bool {
    let this_page_count: u32 = payload
        .results
        .iter()
        .filter(|r| r.error.is_none())
        .map(|r| r.discoveries.len() as u32)
        .sum();

    if let Err(e) = crate::queries::autodiscovery::process_discovery_results(
        state.db(),
        svc.id,
        svc.tenant_id,
        host_id,
        payload,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            service_id = %svc.id,
            "failed to process discovery results"
        );
        return false;
    }

    // Fire software-item lifecycle plugins on newly discovered items that may
    // benefit from enrichment (e.g. icon assignment from Dashboard Icons).
    enrich_discovered_items(state, svc).await;

    match page_outcome {
        PageOutcome::Final {
            accumulated_discovered_count,
        } => {
            let total_discovered = accumulated_discovered_count.saturating_add(this_page_count);
            if total_discovered > 0 {
                let host_name = host::Entity::find_by_id(host_id)
                    .one(state.db())
                    .await
                    .ok()
                    .flatten()
                    .map(|h| h.hostname.clone());

                {
                    let mut event = NotificationEvent::new(
                        svc.tenant_id,
                        NotificationEventDetails::NewSoftwareDiscovered {
                            discovered_count: total_discovered,
                        },
                    );
                    event.host_id = Some(host_id);
                    event.host_name = host_name;
                    state.notification.notification_dispatcher.dispatch(event);
                }
            }
        }
        PageOutcome::Pending => {
            if let Some(p) = pagination {
                report_tracker.add_discovered_count(p.report_id, this_page_count);
            }
        }
    }

    true
}

/// Handle a `DiscoveryResults` message: find host, process results.
#[tracing::instrument(skip_all, fields(%service_id, host_machine_id = %payload.host_machine_id))]
pub(super) async fn handle_discovery_results(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: DiscoveryResultsPayload,
    pagination: Option<&ReportPagination>,
    report_tracker: &mut ReportTracker,
) -> ProcessorResponse {
    tracing::debug!(
        %service_id,
        host_machine_id = %payload.host_machine_id,
        results = payload.results.len(),
        page = pagination.map(|p| p.page),
        total_pages = pagination.map(|p| p.total_pages),
        "received DiscoveryResults"
    );

    let service_model = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc,
        Ok(None) => {
            tracing::warn!(
                %service_id,
                "service not found for DiscoveryResults"
            );
            return ProcessorResponse::cont();
        }
        Err(e) => {
            tracing::warn!(
                %service_id,
                error = %e,
                "failed to resolve service for DiscoveryResults"
            );
            return ProcessorResponse::cont();
        }
    };

    let plugin_results = payload.results.len() as u32;
    let discovered_items_reported: u32 = payload
        .results
        .iter()
        .filter(|result| result.error.is_none())
        .map(|result| result.discoveries.len() as u32)
        .sum();

    // Determine whether this is the final page (or a non-paginated message).
    let page_outcome = if let Some(p) = pagination {
        match report_tracker.register_page(p.report_id, p.page, p.total_pages) {
            Ok(outcome) => outcome,
            Err(e) => {
                tracing::warn!(
                    %service_id,
                    error = %e,
                    "invalid pagination for DiscoveryResults"
                );
                emit_service_inventory_audit(
                    state,
                    &service_model,
                    uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    Some((
                        "service",
                        service_model.id.to_string(),
                        Some(service_model.friendly_name.clone()),
                    )),
                    serde_json::json!({
                        "reason_code": "invalid_pagination",
                        "host_machine_id": payload.host_machine_id,
                        "plugin_results": plugin_results,
                        "discovered_items_reported": discovered_items_reported,
                        "page": p.page,
                        "total_pages": p.total_pages,
                        "report_id": p.report_id,
                    }),
                );
                return ProcessorResponse::cont();
            }
        }
    } else {
        // Non-paginated: treat as final (and only) page.
        PageOutcome::Final {
            accumulated_discovered_count: 0,
        }
    };

    let links = service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(service_id))
        .all(state.db())
        .await
        .unwrap_or_default();

    let host_machine_id = payload.host_machine_id.clone();
    match find_linked_host_by_machine_id(state.db(), &links, &host_machine_id).await {
        Some(host_id) => {
            let processed = process_discovery_page_for_host(
                state,
                &service_model,
                host_id,
                payload,
                page_outcome,
                pagination,
                report_tracker,
            )
            .await;
            let host_display = host::Entity::find_by_id(host_id)
                .one(state.db())
                .await
                .ok()
                .flatten()
                .map(|host| host.friendly_name);

            if !processed || discovered_items_reported > 0 {
                emit_service_inventory_audit(
                    state,
                    &service_model,
                    uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
                    if processed {
                        uptrakit_audit_log::AuditOutcome::Success
                    } else {
                        uptrakit_audit_log::AuditOutcome::Failed
                    },
                    Some(("host", host_id.to_string(), host_display)),
                    serde_json::json!({
                        "host_machine_id": host_machine_id,
                        "plugin_results": plugin_results,
                        "discovered_items_reported": discovered_items_reported,
                        "paginated": pagination.is_some(),
                        "page": pagination.map(|p| p.page),
                        "total_pages": pagination.map(|p| p.total_pages),
                        "reason_code": if processed {
                            serde_json::Value::Null
                        } else {
                            serde_json::Value::String("process_discovery_results_failed".to_string())
                        },
                    }),
                );
            }
        }
        None => {
            tracing::warn!(
                %service_id,
                host_machine_id = %host_machine_id,
                "received DiscoveryResults for unknown host machine_id"
            );
            emit_service_inventory_audit(
                state,
                &service_model,
                uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some((
                    "service",
                    service_model.id.to_string(),
                    Some(service_model.friendly_name.clone()),
                )),
                serde_json::json!({
                    "reason_code": "unknown_host_machine_id",
                    "host_machine_id": host_machine_id,
                    "plugin_results": plugin_results,
                    "discovered_items_reported": discovered_items_reported,
                    "paginated": pagination.is_some(),
                    "page": pagination.map(|p| p.page),
                    "total_pages": pagination.map(|p| p.total_pages),
                }),
            );
        }
    }

    ProcessorResponse::cont()
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
// Software-item lifecycle enrichment (post-discovery)
// ---------------------------------------------------------------------------

/// After discovery results are processed, fire lifecycle plugins on featured
/// icon-less items. This is a best-effort operation — errors on individual
/// items are logged but never propagate.
async fn enrich_discovered_items(state: &AppState, service_model: &service::Model) {
    let tenant_id = service_model.tenant_id;
    let items =
        crate::queries::software_items::load_items_needing_enrichment(state.db(), tenant_id).await;

    let lifecycle_ctx = match crate::queries::plugin_type_settings::preload_lifecycle_type_settings(
        state.db(),
        tenant_id,
        state.plugin.plugin_ops.as_ref(),
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!(
                error = %e,
                %tenant_id,
                "failed to preload lifecycle type settings; using defaults"
            );
            uptrakit_plugin_infrastructure_registry::SoftwareItemLifecycleContext::default()
        }
    };

    tracing::debug!(%tenant_id, count = items.len(), "lifecycle enrichment loaded items");
    let examined_count = items.len() as u32;
    let mut patch_attempt_count = 0u32;
    let mut patched_count = 0u32;
    let mut patch_failed_count = 0u32;

    for item in items {
        let event = uptrakit_plugin_infrastructure_registry::SoftwareItemCreatedEvent::new(
            item.id,
            item.tenant_id,
            item.name.clone(),
            item.featured,
            item.icon_url.clone(),
        );
        match state
            .plugin
            .plugin_ops
            .on_software_item_created(&event, &lifecycle_ctx)
            .await
        {
            Some(patch) => {
                patch_attempt_count += 1;
                if let Err(e) = crate::queries::software_items::apply_software_item_patch(
                    state.db(),
                    item.id,
                    &patch,
                )
                .await
                {
                    tracing::warn!(
                        error = %e,
                        item_id = %item.id,
                        name = %item.name,
                        "lifecycle patch failed"
                    );
                    patch_failed_count += 1;
                } else {
                    tracing::trace!(item_id = %item.id, name = %item.name, "lifecycle patch applied");
                    patched_count += 1;
                }
            }
            None => {
                tracing::trace!(item_id = %item.id, name = %item.name, "lifecycle plugin produced no patch");
            }
        }
    }

    if patch_attempt_count > 0 {
        let outcome = if patch_failed_count == 0 {
            uptrakit_audit_log::AuditOutcome::Success
        } else if patched_count > 0 {
            uptrakit_audit_log::AuditOutcome::Partial
        } else {
            uptrakit_audit_log::AuditOutcome::Failed
        };
        emit_service_inventory_audit(
            state,
            service_model,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_ENRICH,
            outcome,
            Some((
                "service",
                service_model.id.to_string(),
                Some(service_model.friendly_name.clone()),
            )),
            serde_json::json!({
                "examined_count": examined_count,
                "patch_attempt_count": patch_attempt_count,
                "patched_count": patched_count,
                "patch_failed_count": patch_failed_count,
            }),
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod tests;
