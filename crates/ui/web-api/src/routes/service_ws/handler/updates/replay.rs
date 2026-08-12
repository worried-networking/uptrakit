//! Reconnect recovery + pending-update replay preparation.

#![expect(clippy::indexing_slicing, reason = "index is computed to be in bounds")]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use time::OffsetDateTime;

use rootcause::prelude::*;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, service, software_item,
    update_history,
};
use uptrakit_wire::{ControllerMessage, ExecuteUpdatePayload, PluginAssignment};

use super::dispatch::notify_failed_reconnect_update;
use super::finalize::finalize_post_update_best_effort;
use super::{HandlerError, HandlerResult, load_linked_host_ids};
use super::{RECOVERY_FINALIZATION_TIMEOUT, ReconnectSuccessorDispatchMode};
use crate::AppState;

/// Result of preparing one pending record during reconnect replay.
enum PendingRecordOutcome {
    /// Replay this message to the agent.
    Message(Box<ExecuteUpdatePayload>),
    /// Skip silently (already-dispatched batch host, or orchestrator spawned).
    Skip,
    /// The record could not be reconstructed and was failed; retry the outer loop.
    Failed,
}

/// Prepare a single pending record. Mirrors the original inline loop body
/// exactly — no behavior change.
async fn prepare_single_pending_record(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    update_record: &update_history::Model,
    records: &PendingUpdateRecords,
    dispatched_batch_hosts: &mut HashSet<(uuid::Uuid, uuid::Uuid)>,
) -> HandlerResult<PendingRecordOutcome> {
    // Unprotected Pending -> spawn orchestrator instead of direct replay.
    if update_record.pre_update_protection_status.is_none() {
        let target = match crate::queries::update_dispatch::load_target_for_dispatch(
            state.db(),
            update_record.tenant_id,
            update_record.host_id,
            update_record.software_item_id,
        )
        .await
        {
            Ok(target) => target,
            Err(e) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    error = %e,
                    "could not load target for unprotected Pending update on reconnect; failing record"
                );
                if fail_unreplayable_pending_update(state, service_id, update_record).await? {
                    return Ok(PendingRecordOutcome::Failed);
                }
                return Ok(PendingRecordOutcome::Skip);
            }
        };
        let work = crate::queries::update_triggers::PendingProtectionWork {
            target,
            update_history_id: update_record.id,
            to_version: update_record.to_version.clone().unwrap_or_default(),
            release_info: None,
            interactive: update_record.interactive,
        };
        state.update_dispatcher.spawn_pending_protection(work);
        return Ok(PendingRecordOutcome::Skip);
    }

    if let Some(batch_id) = update_record.batch_id {
        let key = (batch_id, update_record.host_id);
        if !dispatched_batch_hosts.insert(key) {
            return Ok(PendingRecordOutcome::Skip);
        }
    }

    let Some(execute_payload) = build_execute_payload(update_record, records) else {
        if fail_unreplayable_pending_update(state, service_id, update_record).await? {
            return Ok(PendingRecordOutcome::Failed);
        }
        return Ok(PendingRecordOutcome::Skip);
    };

    tracing::info!(
        update_id = %update_record.id,
        %service_id,
        software = %records
            .sw_items_map
            .get(&update_record.software_item_id)
            .map(|i| i.name.as_str())
            .unwrap_or("?"),
        "prepared pending update on reconnect"
    );
    Ok(PendingRecordOutcome::Message(Box::new(execute_payload)))
}

/// All data loaded from the DB that is needed to dispatch pending updates for
/// a set of hosts.
pub(in super::super) struct PendingUpdateRecords {
    pending_updates: Vec<update_history::Model>,
    sw_items_map: HashMap<uuid::Uuid, software_item::Model>,
    hosts_map: HashMap<uuid::Uuid, host::Model>,
    assignments_map: HashMap<(uuid::Uuid, uuid::Uuid, String), host_software_item_plugin::Model>,
    hook_assignments_map:
        HashMap<(uuid::Uuid, uuid::Uuid, String), Vec<host_software_item_plugin::Model>>,
    configs_map: HashMap<uuid::Uuid, plugin_config::Model>,
    hsi_metadata_map: HashMap<(uuid::Uuid, uuid::Uuid), Option<serde_json::Value>>,
}

/// Load all pending update records and their auxiliary data for hosts linked to
/// `service_id`.
///
/// Returns `None` when there are no host links or no pending records (nothing
/// to dispatch).
pub(in super::super) async fn load_pending_update_records(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
) -> HandlerResult<Option<(Vec<uuid::Uuid>, PendingUpdateRecords)>> {
    // 1. Find host_ids linked to this service.
    let host_ids: Vec<uuid::Uuid> = load_linked_host_ids(state.db(), service_id)
        .await?
        .into_iter()
        .collect();

    if host_ids.is_empty() {
        return Ok(None);
    }

    // 2. Query pending update_history records for those hosts.
    //    Ordered by ID (UUIDv7 = chronological) so batch-aware filtering
    //    below picks the oldest pending update per (batch_id, host_id).
    let pending_updates = update_history::Entity::find()
        .filter(update_history::Column::HostId.is_in(host_ids.iter().copied()))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .order_by_asc(update_history::Column::Id)
        .all(state.db())
        .await
        .context_to::<HandlerError>()?;

    if pending_updates.is_empty() {
        return Ok(None);
    }

    // Collect unique IDs needed for batch queries.
    let sw_ids: Vec<uuid::Uuid> = pending_updates
        .iter()
        .map(|u| u.software_item_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // Batch 1: software items.
    let sw_items_map: HashMap<uuid::Uuid, software_item::Model> = software_item::Entity::find()
        .filter(software_item::Column::Id.is_in(sw_ids.iter().copied()))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(state.db())
        .await
        .context_to::<HandlerError>()?
        .into_iter()
        .map(|i| (i.id, i))
        .collect();

    // Batch 2: hosts.
    let hosts_map: HashMap<uuid::Uuid, host::Model> = host::Entity::find()
        .filter(host::Column::Id.is_in(host_ids.iter().copied()))
        .all(state.db())
        .await
        .context_to::<HandlerError>()?
        .into_iter()
        .map(|h| (h.id, h))
        .collect();

    // Batch 3: plugin assignments for all relevant roles across all
    // (host_id, software_item_id) combinations that appear in pending_updates.
    // The cross-product filter may include extra rows for pairs not in
    // pending_updates; those are silently ignored during the join below.
    let assignments: Vec<host_software_item_plugin::Model> =
        host_software_item_plugin::Entity::find()
            .filter(host_software_item_plugin::Column::HostId.is_in(host_ids.iter().copied()))
            .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(sw_ids.iter().copied()))
            .filter(host_software_item_plugin::Column::Role.is_in([
                "execute_update",
                "detect_version",
                "fetch_releases",
                "pre_update_hook",
                "post_update_hook",
            ]))
            .order_by_asc(host_software_item_plugin::Column::Ordinal)
            .all(state.db())
            .await
            .context_to::<HandlerError>()?;

    // Index single-valued assignments by (host_id, software_item_id, role).
    // Hook roles are collected separately as Vec since multiple can exist.
    let mut assignments_map: HashMap<
        (uuid::Uuid, uuid::Uuid, String),
        host_software_item_plugin::Model,
    > = HashMap::new();
    let mut hook_assignments_map: HashMap<
        (uuid::Uuid, uuid::Uuid, String),
        Vec<host_software_item_plugin::Model>,
    > = HashMap::new();
    for a in assignments {
        let key = (a.host_id, a.software_item_id, a.role.clone());
        if a.role == "pre_update_hook" || a.role == "post_update_hook" {
            hook_assignments_map.entry(key).or_default().push(a);
        } else {
            assignments_map.insert(key, a);
        }
    }

    // Batch 4: plugin configs referenced by the assignments above.
    let plugin_config_ids: Vec<uuid::Uuid> = assignments_map
        .values()
        .filter_map(|a| a.plugin_config_id)
        .chain(
            hook_assignments_map
                .values()
                .flatten()
                .filter_map(|a| a.plugin_config_id),
        )
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let configs_map: HashMap<uuid::Uuid, plugin_config::Model> = plugin_config::Entity::find()
        .filter(plugin_config::Column::Id.is_in(plugin_config_ids))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .all(state.db())
        .await
        .context_to::<HandlerError>()?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();

    // Batch 5: host_software_item rows for `latest_release_metadata`.
    //
    // The cross-product filter may return extra rows; they are silently
    // ignored during the join below (same pattern as batch 3).
    // This allows `enrich_release_info_with_attestation` to reconstruct
    // `release_info` for plugins like GitHub that require asset download URLs.
    let hsi_metadata_map: HashMap<(uuid::Uuid, uuid::Uuid), Option<serde_json::Value>> =
        host_software_item::Entity::find()
            .filter(host_software_item::Column::HostId.is_in(host_ids.iter().copied()))
            .filter(host_software_item::Column::SoftwareItemId.is_in(sw_ids))
            .all(state.db())
            .await
            .context_to::<HandlerError>()?
            .into_iter()
            .map(|m| ((m.host_id, m.software_item_id), m.latest_release_metadata))
            .collect();

    Ok(Some((
        host_ids,
        PendingUpdateRecords {
            pending_updates,
            sw_items_map,
            hosts_map,
            assignments_map,
            hook_assignments_map,
            configs_map,
            hsi_metadata_map,
        },
    )))
}

pub(in super::super) async fn recover_owned_updates_on_connect_with_dispatch_mode(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    runtime_instance_id: Option<uuid::Uuid>,
    successor_dispatch_mode: ReconnectSuccessorDispatchMode,
) -> HandlerResult<()> {
    let failed =
        match crate::queries::update_batches::mark_owned_in_progress_as_interrupted_on_reconnect(
            state.db(),
            service_id,
            runtime_instance_id,
        )
        .await
        {
            Ok(failed) => failed,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    %service_id,
                    "failed owner-aware reconnect cleanup query"
                );
                bail!(HandlerError::WebSocketSend);
            }
        };

    let linked_host_ids_for_orchestrator: Vec<uuid::Uuid> =
        load_linked_host_ids(state.db(), service_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    if let Err(e) =
        crate::queries::update_batches::mark_orchestrator_inprogress_as_failed_on_reconnect(
            state.db(),
            &linked_host_ids_for_orchestrator,
        )
        .await
    {
        tracing::warn!(
            %service_id,
            error = %e,
            "failed to mark orchestrator-owned InProgress records as Failed on reconnect"
        );
    }

    if failed.is_empty() {
        return Ok(());
    }

    let reason = "Update interrupted: agent restarted".to_string();
    for record in &failed {
        finalize_post_update_best_effort(state, record, Some(RECOVERY_FINALIZATION_TIMEOUT)).await;
        notify_failed_reconnect_update(
            state,
            service_id,
            record.tenant_id,
            record,
            &reason,
            successor_dispatch_mode,
        )
        .await;
    }

    state
        .notification
        .notification_service
        .push_software_states_for_tenant(state.db(), failed[0].tenant_id)
        .await;

    Ok(())
}

pub(in super::super) async fn prepare_pending_replay_messages(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
) -> HandlerResult<Vec<ControllerMessage>> {
    loop {
        let Some((_host_ids, records)) = load_pending_update_records(state, service_id).await?
        else {
            return Ok(Vec::new());
        };

        tracing::info!(
            %service_id,
            count = records.pending_updates.len(),
            "preparing pending updates on reconnect"
        );

        let mut dispatched_batch_hosts: HashSet<(uuid::Uuid, uuid::Uuid)> = HashSet::new();
        let mut failed_any = false;
        let mut messages = Vec::new();

        for update_record in &records.pending_updates {
            match prepare_single_pending_record(
                state,
                service_id,
                update_record,
                &records,
                &mut dispatched_batch_hosts,
            )
            .await?
            {
                PendingRecordOutcome::Message(payload) => {
                    messages.push(ControllerMessage::ExecuteUpdate(payload));
                }
                PendingRecordOutcome::Skip => {}
                PendingRecordOutcome::Failed => failed_any = true,
            }
        }

        if !failed_any {
            return Ok(messages);
        }
    }
}

async fn fail_unreplayable_pending_update(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    update_record: &update_history::Model,
) -> HandlerResult<bool> {
    let reason =
        "Update replay failed: controller could not reconstruct the pending update after reconnect"
            .to_string();
    let completed_at = OffsetDateTime::now_utc();
    let update_result = update_history::Entity::update_many()
        .filter(update_history::Column::Id.eq(update_record.id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::Failed),
        )
        .col_expr(
            update_history::Column::CompletedAt,
            Expr::value(Some(completed_at)),
        )
        .col_expr(update_history::Column::Output, Expr::value(reason.clone()))
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(reason.len() as i64),
        )
        .exec(state.db())
        .await
        .context_to::<HandlerError>()?;

    if update_result.rows_affected == 0 {
        return Ok(false);
    }

    let mut failed_record = update_record.clone();
    failed_record.status = update_history::UpdateStatus::Failed;
    failed_record.completed_at = Some(completed_at);
    failed_record.output = reason.clone();
    failed_record.output_bytes = reason.len() as i64;

    finalize_post_update_best_effort(state, &failed_record, None).await;

    let tenant_id = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
        .context_to::<HandlerError>()?
        .map(|svc| svc.tenant_id)
        .unwrap_or(update_record.tenant_id);

    notify_failed_reconnect_update(
        state,
        service_id,
        tenant_id,
        &failed_record,
        &reason,
        ReconnectSuccessorDispatchMode::ReplayPrepared,
    )
    .await;

    state
        .notification
        .notification_service
        .push_software_states_for_tenant(state.db(), tenant_id)
        .await;

    Ok(true)
}

/// Build an [`ExecuteUpdatePayload`] for a single pending update record using
/// the preloaded lookup maps.
///
/// Returns `None` and logs a warning for any unresolvable dependency (missing
/// software item, missing host, unknown plugin type, etc.) so the caller can
/// skip the record.
fn build_execute_payload(
    update_record: &update_history::Model,
    records: &PendingUpdateRecords,
) -> Option<ExecuteUpdatePayload> {
    let item = match records.sw_items_map.get(&update_record.software_item_id) {
        Some(i) => i,
        None => {
            tracing::warn!(
                update_id = %update_record.id,
                software_item_id = %update_record.software_item_id,
                "software item not found or deactivated, skipping pending update"
            );
            return None;
        }
    };

    // Resolve execute_update assignment.
    let exec_key = (update_record.host_id, item.id, "execute_update".to_string());
    let exec_assignment = match records.assignments_map.get(&exec_key) {
        Some(a) => a,
        None => {
            tracing::warn!(
                update_id = %update_record.id,
                host_id = %update_record.host_id,
                software_item_id = %item.id,
                "no execute_update plugin assigned, skipping pending update"
            );
            return None;
        }
    };
    let exec_config = exec_assignment
        .plugin_config_id
        .and_then(|pc_id| records.configs_map.get(&pc_id));

    let execute_update_plugin = match build_plugin_assignment_nullable(exec_assignment, exec_config)
    {
        Some(a) => a,
        None => {
            tracing::warn!(
                update_id = %update_record.id,
                "unknown plugin type for execute_update, skipping pending update"
            );
            return None;
        }
    };

    // Resolve optional detect_version assignment.
    let detect_key = (update_record.host_id, item.id, "detect_version".to_string());
    let detect_version_plugin = records.assignments_map.get(&detect_key).and_then(|a| {
        let c = a
            .plugin_config_id
            .and_then(|pc_id| records.configs_map.get(&pc_id));
        build_plugin_assignment_nullable(a, c)
    });

    // Resolve hook plugin assignments.
    let pre_hook_key = (
        update_record.host_id,
        update_record.software_item_id,
        "pre_update_hook".to_string(),
    );
    let pre_update_hook_plugins: Vec<PluginAssignment> = records
        .hook_assignments_map
        .get(&pre_hook_key)
        .map(|assignments| {
            assignments
                .iter()
                .filter_map(|a| {
                    let c = a
                        .plugin_config_id
                        .and_then(|pc_id| records.configs_map.get(&pc_id));
                    build_plugin_assignment_nullable(a, c)
                })
                .collect()
        })
        .unwrap_or_default();
    let post_hook_key = (
        update_record.host_id,
        update_record.software_item_id,
        "post_update_hook".to_string(),
    );
    let post_update_hook_plugins: Vec<PluginAssignment> = records
        .hook_assignments_map
        .get(&post_hook_key)
        .map(|assignments| {
            assignments
                .iter()
                .filter_map(|a| {
                    let c = a
                        .plugin_config_id
                        .and_then(|pc_id| records.configs_map.get(&pc_id));
                    build_plugin_assignment_nullable(a, c)
                })
                .collect()
        })
        .unwrap_or_default();

    let host = match records.hosts_map.get(&update_record.host_id) {
        Some(h) => h,
        None => {
            tracing::warn!(
                update_id = %update_record.id,
                host_id = %update_record.host_id,
                "host not found for pending update, skipping"
            );
            return None;
        }
    };

    // Reconstruct release_info from latest_release_metadata so that
    // asset-download plugins (e.g. GitHub) receive the download URLs on
    // reconnect replay — same enrichment used in dispatch_update_to_agent.
    let hsi_metadata = records
        .hsi_metadata_map
        .get(&(update_record.host_id, item.id))
        .and_then(|m| m.as_ref());
    let fetch_key = (update_record.host_id, item.id, "fetch_releases".to_string());
    let fetch_config = records.assignments_map.get(&fetch_key).map(|assignment| {
        let config = assignment
            .plugin_config_id
            .and_then(|pc_id| records.configs_map.get(&pc_id));
        merged_plugin_config(assignment, config)
    });
    let release_info = crate::queries::update_dispatch::enrich_release_info_with_attestation(
        None,
        hsi_metadata,
        fetch_config.as_ref(),
    );

    Some(ExecuteUpdatePayload {
        host_machine_id: host.machine_id.clone(),
        update_history_id: update_record.id,
        software_item_id: item.id,
        software_item_name: item.name.clone(),
        to_version: update_record.to_version.clone().unwrap_or_default(),
        detect_version_plugin,
        execute_update_plugin,
        pre_update_hook_plugins,
        post_update_hook_plugins,
        release_info,
        timeout: uptrakit_wire::DEFAULT_UPDATE_TIMEOUT,
        // Preserve the interactive flag that was set at original dispatch time
        // so that a reconnecting agent receives a PTY when expected.
        interactive: update_record.interactive,
    })
}

// ---------------------------------------------------------------------------
// Role-based plugin helpers
// ---------------------------------------------------------------------------

/// Build a [`PluginAssignment`] from a role plugin row and its optional config.
///
/// When `config` is `None` (no stored `plugin_config` linked to the assignment),
/// the plugin type is read from `assignment.plugin_type` and the effective config
/// is built from the assignment-level override alone.
///
/// Returns `None` if the plugin type string cannot be deserialized.
fn merged_plugin_config(
    assignment: &host_software_item_plugin::Model,
    config: Option<&plugin_config::Model>,
) -> serde_json::Value {
    uptrakit_config_merge::resolve_effective_config(
        None,
        config.map(|c| c.config.as_json()),
        assignment.config.as_ref(),
    )
}

fn build_plugin_assignment_nullable(
    assignment: &host_software_item_plugin::Model,
    config: Option<&plugin_config::Model>,
) -> Option<PluginAssignment> {
    let plugin_type_str = config
        .map(|c| c.plugin_type.clone())
        .unwrap_or_else(|| assignment.plugin_type.clone());
    let plugin_type = uptrakit_shared_types::PluginTypeId::new(plugin_type_str);
    let merged_config = merged_plugin_config(assignment, config);

    Some(PluginAssignment {
        plugin_type,
        package_identifier: assignment.package_identifier.clone(),
        config: merged_config,
    })
}
