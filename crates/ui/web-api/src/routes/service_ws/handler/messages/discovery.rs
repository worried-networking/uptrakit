use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use uptrakit_shared_db::entity::{host, service, service_host};
use uptrakit_wire::report_tracker::{PageOutcome, ReportTracker};
use uptrakit_wire::{DiscoveryResultsPayload, ReportPagination};

use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};

use super::ProcessorResponse;
use super::emit_service_inventory_audit;

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
pub(in super::super) async fn handle_discovery_results(
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
// Software-item lifecycle enrichment (post-discovery)
// ---------------------------------------------------------------------------

/// After discovery results are processed, fire lifecycle plugins on featured
/// icon-less items. This is a best-effort operation — errors on individual
/// items are logged but never propagate.
pub(super) async fn enrich_discovered_items(state: &AppState, service_model: &service::Model) {
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
