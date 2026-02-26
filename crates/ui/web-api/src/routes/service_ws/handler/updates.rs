//! Update delivery, ownership validation, and update-lifecycle message handlers.
//!
//! Contains `validate_update_ownership`, `load_linked_host_ids`,
//! `deliver_pending_updates`, and the per-message handlers
//! `handle_update_started`, `handle_update_output`, and `handle_update_result`.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use sea_orm::sea_query::{Expr, ExprTrait};

use rootcause::prelude::*;
use uptrakit_internal_wire::{
    ControllerMessage, ExecuteUpdatePayload, OutgoingSeq, PluginAssignment,
    UpdateFinalStatus, UpdateOutputPayload, UpdateResultPayload, UpdateStartedPayload,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, service, service_host,
    software_item, update_history, update_output_line,
};

use super::{HandlerError, HandlerResult, LoopAction, MAX_UPDATE_OUTPUT_BYTES};
use crate::routes::service_ws::protocol::serialize_controller_msg;
use crate::AppState;

// ---------------------------------------------------------------------------
// load_linked_host_ids
// ---------------------------------------------------------------------------

/// Load the set of host IDs linked to the given service.
pub(super) async fn load_linked_host_ids(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
) -> HandlerResult<HashSet<uuid::Uuid>> {
    let links = service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(service_id))
        .all(db)
        .await
        .context_to::<HandlerError>()?;

    Ok(links.into_iter().map(|l| l.host_id).collect())
}

// ---------------------------------------------------------------------------
// validate_update_ownership
// ---------------------------------------------------------------------------

/// Validate that an `update_history` record belongs to a host linked to the
/// current service. Returns the record on success, logs a warning and returns
/// an error if the service does not own the record.
pub(super) async fn validate_update_ownership(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    update_history_id: uuid::Uuid,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> HandlerResult<update_history::Model> {
    let record = uptrakit_shared_db::entity::prelude::UpdateHistory::find_by_id(update_history_id)
        .one(db)
        .await
        .context_to::<HandlerError>()?
        .ok_or_else(|| {
            tracing::warn!(
                %service_id,
                update_id = %update_history_id,
                "update_history record not found"
            );
            report!(HandlerError::WebSocketSend)
        })?;

    if !linked_host_ids.contains(&record.host_id) {
        tracing::warn!(
            %service_id,
            update_id = %update_history_id,
            host_id = %record.host_id,
            "service attempted to update record for unlinked host"
        );
        bail!(HandlerError::WebSocketSend);
    }

    Ok(record)
}

// ---------------------------------------------------------------------------
// deliver_pending_updates
// ---------------------------------------------------------------------------

/// Deliver pending updates for hosts linked to this service.
///
/// On service reconnect, we check for any `update_history` records with
/// `status = Pending` for hosts linked to this service and send them.
pub(super) async fn deliver_pending_updates(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
) -> HandlerResult<()> {
    // 1. Find host_ids linked to this service.
    let host_links = service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(service_id))
        .all(state.db())
        .await
        .context_to::<HandlerError>()?;

    if host_links.is_empty() {
        return Ok(());
    }

    let host_ids: Vec<uuid::Uuid> = host_links.iter().map(|l| l.host_id).collect();

    // 2. Query pending update_history records for those hosts.
    let pending_updates = update_history::Entity::find()
        .filter(update_history::Column::HostId.is_in(host_ids))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .all(state.db())
        .await
        .context_to::<HandlerError>()?;

    if pending_updates.is_empty() {
        return Ok(());
    }

    tracing::info!(
        %service_id,
        count = pending_updates.len(),
        "delivering pending updates on reconnect"
    );

    // 3. Build ExecuteUpdatePayload for each and send.
    for update_record in pending_updates {
        let item = match software_item::Entity::find_by_id(update_record.software_item_id)
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(state.db())
            .await
        {
            Ok(Some(i)) => i,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    software_item_id = %update_record.software_item_id,
                    "software item not found or deactivated, skipping pending update"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load software item for pending update");
                continue;
            }
        };

        // Load role-specific plugin assignments for this host-software pair.
        let execute_update_assignment = match load_role_plugin(
            state.db(),
            update_record.host_id,
            item.id,
            "execute_update",
        )
        .await
        {
            Ok(Some(data)) => data,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    host_id = %update_record.host_id,
                    software_item_id = %item.id,
                    "no execute_update plugin assigned, skipping pending update"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to load execute_update plugin for pending update"
                );
                continue;
            }
        };

        let detect_version_assignment =
            match load_role_plugin(state.db(), update_record.host_id, item.id, "detect_version")
                .await
            {
                Ok(data) => data,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to load detect_version plugin for pending update"
                    );
                    None
                }
            };

        let execute_update_plugin =
            match build_plugin_assignment(&execute_update_assignment.0, &execute_update_assignment.1)
            {
                Some(a) => a,
                None => {
                    tracing::warn!(
                        update_id = %update_record.id,
                        "unknown plugin type for execute_update, skipping pending update"
                    );
                    continue;
                }
            };

        let detect_version_plugin = detect_version_assignment
            .as_ref()
            .and_then(|(a, c)| build_plugin_assignment(a, c));

        // Resolve hooks from the execute_update plugin config + per-role override.
        let resolved_hooks = crate::update_hooks::resolve_hooks(
            &execute_update_assignment.1.config,
            execute_update_assignment.0.config_override.as_ref(),
        );

        // Look up the host's machine_id so the agent can route correctly.
        let host_machine_id = match host::Entity::find_by_id(update_record.host_id)
            .one(state.db())
            .await
        {
            Ok(Some(h)) => h.machine_id,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    host_id = %update_record.host_id,
                    "host not found for pending update, skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load host for pending update");
                continue;
            }
        };

        let execute_payload = ExecuteUpdatePayload {
            host_machine_id,
            update_history_id: update_record.id,
            software_item_id: item.id,
            software_item_name: item.name.clone(),
            to_version: update_record.to_version.clone(),
            detect_version_plugin,
            execute_update_plugin,
            pre_update_hooks: resolved_hooks.pre_update_hooks,
            post_update_hooks: resolved_hooks.post_update_hooks,
            release_info: None,
            timeout_seconds: 300,
        };

        let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
        let Some(json) = serialize_controller_msg(out_seq, msg) else {
            continue;
        };

        if sink.send(Message::Text(json.into())).await.is_err() {
            bail!(HandlerError::WebSocketSend);
        }

        tracing::info!(
            update_id = %update_record.id,
            %service_id,
            software = %item.name,
            "delivered pending update on reconnect"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// handle_update_started
// ---------------------------------------------------------------------------

/// Handle an `UpdateStarted` message: validate ownership, set status to
/// `InProgress`, clear previous output lines.
pub(super) async fn handle_update_started(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateStartedPayload,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> LoopAction {
    tracing::info!(
        update_id = %payload.update_history_id,
        from_version = ?payload.from_version,
        "update started"
    );
    let record = match validate_update_ownership(
        state.db(),
        service_id,
        payload.update_history_id,
        linked_host_ids,
    )
    .await
    {
        Ok(r) => r,
        Err(_) => return LoopAction::Continue,
    };
    let mut active: update_history::ActiveModel = record.into();
    active.status = Set(update_history::UpdateStatus::InProgress);
    active.started_at = Set(time::OffsetDateTime::now_utc());
    if payload.from_version.is_some() {
        active.from_version = Set(payload.from_version.clone());
    }
    active.output = Set(String::new());
    active.output_bytes = Set(0);
    if let Err(e) = active.update(state.db()).await {
        tracing::warn!(
            error = %e,
            "failed to update update_history status"
        );
    }
    if let Err(e) = update_output_line::Entity::delete_many()
        .filter(
            update_output_line::Column::UpdateHistoryId.eq(payload.update_history_id),
        )
        .exec(state.db())
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to clear update output lines"
        );
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_update_output
// ---------------------------------------------------------------------------

/// Handle an `UpdateOutput` message: validate ownership, append output line
/// (capped at `MAX_UPDATE_OUTPUT_BYTES`).
pub(super) async fn handle_update_output(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateOutputPayload,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> LoopAction {
    tracing::trace!(
        update_id = %payload.update_history_id,
        stream = ?payload.stream,
        "update output"
    );
    if validate_update_ownership(
        state.db(),
        service_id,
        payload.update_history_id,
        linked_host_ids,
    )
    .await
    .is_err()
    {
        return LoopAction::Continue;
    }

    let output_line = format!("{}\n", payload.output);
    let line_len = output_line.len() as i64;
    let updated = update_history::Entity::update_many()
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::col(update_history::Column::OutputBytes).add(line_len),
        )
        .filter(update_history::Column::Id.eq(payload.update_history_id))
        .filter(
            update_history::Column::OutputBytes.lt(MAX_UPDATE_OUTPUT_BYTES as i64),
        )
        .exec(state.db())
        .await;

    let Ok(updated) = updated else {
        tracing::warn!(
            update_id = %payload.update_history_id,
            "failed to update output bytes"
        );
        return LoopAction::Continue;
    };

    if updated.rows_affected == 0 {
        tracing::debug!(
            update_id = %payload.update_history_id,
            "update output exceeded {MAX_UPDATE_OUTPUT_BYTES} byte cap, dropping"
        );
        return LoopAction::Continue;
    }

    let line = update_output_line::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        update_history_id: Set(payload.update_history_id),
        stream: Set(payload.stream),
        output: Set(output_line),
        created_at: Set(time::OffsetDateTime::now_utc()),
    };
    if let Err(e) = update_output_line::Entity::insert(line)
        .exec(state.db())
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to insert update output line"
        );
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_update_result
// ---------------------------------------------------------------------------

/// Handle an `UpdateResult` message: validate ownership, set final status,
/// store output, update installed version on success, push software states.
pub(super) async fn handle_update_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: UpdateResultPayload,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> LoopAction {
    tracing::info!(
        update_id = %payload.update_history_id,
        status = ?payload.status,
        error = ?payload.error,
        "update result"
    );
    let record = match validate_update_ownership(
        state.db(),
        service_id,
        payload.update_history_id,
        linked_host_ids,
    )
    .await
    {
        Ok(r) => r,
        Err(_) => return LoopAction::Continue,
    };
    let mut active: update_history::ActiveModel = record.clone().into();
    active.status = Set(match payload.status {
        UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
        UpdateFinalStatus::Failed => update_history::UpdateStatus::Failed,
        _ => update_history::UpdateStatus::Failed,
    });
    active.completed_at = Set(Some(time::OffsetDateTime::now_utc()));
    let capped_output = if payload.output.len() > MAX_UPDATE_OUTPUT_BYTES {
        payload.output[..MAX_UPDATE_OUTPUT_BYTES].to_string()
    } else {
        payload.output
    };
    active.output = Set(capped_output.clone());
    active.output_bytes = Set(capped_output.len() as i64);
    if payload.from_version.is_some() {
        active.from_version = Set(payload.from_version);
    }
    if let Err(e) = active.update(state.db()).await {
        tracing::warn!(
            error = %e,
            "failed to update update_history result"
        );
    }

    if let Err(e) = update_output_line::Entity::delete_many()
        .filter(
            update_output_line::Column::UpdateHistoryId.eq(payload.update_history_id),
        )
        .exec(state.db())
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to clear update output lines"
        );
    }

    if payload.status == UpdateFinalStatus::Completed
        && let Some(ref to_version) = payload.to_version
        && let Ok(Some(link)) = host_software_item::Entity::find_by_id((
            record.host_id,
            record.software_item_id,
        ))
        .one(state.db())
        .await
    {
        let mut link_active: host_software_item::ActiveModel = link.into();
        link_active.installed_version = Set(Some(to_version.clone()));
        link_active.installed_version_detected_at =
            Set(Some(time::OffsetDateTime::now_utc()));
        link_active.last_updated_at = Set(Some(time::OffsetDateTime::now_utc()));
        if let Err(e) = link_active.update(state.db()).await {
            tracing::warn!(
                error = %e,
                "failed to update host_software_item installed_version"
            );
        }
    }

    // Push updated software states to MQTT services.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification_service
            .push_software_states_for_tenant(svc.tenant_id)
            .await;
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// Role-based plugin helpers
// ---------------------------------------------------------------------------

/// Load a role-specific plugin assignment for a host-software pair.
///
/// Returns `None` if no assignment with the given role exists.
async fn load_role_plugin(
    db: &sea_orm::DatabaseConnection,
    host_id: uuid::Uuid,
    software_item_id: uuid::Uuid,
    role: &str,
) -> HandlerResult<Option<(host_software_item_plugin::Model, plugin_config::Model)>> {
    let assignment = host_software_item_plugin::Entity::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role))
        .one(db)
        .await
        .context_to::<HandlerError>()?;

    let Some(assignment) = assignment else {
        return Ok(None);
    };

    let config = plugin_config::Entity::find_by_id(assignment.plugin_config_id)
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to::<HandlerError>()?;

    let Some(config) = config else {
        tracing::warn!(
            plugin_config_id = %assignment.plugin_config_id,
            role,
            "plugin config not found or deactivated"
        );
        return Ok(None);
    };

    Ok(Some((assignment, config)))
}

/// Build a [`PluginAssignment`] from a role plugin row and its plugin config.
///
/// Returns `None` if the plugin type string cannot be deserialized.
fn build_plugin_assignment(
    assignment: &host_software_item_plugin::Model,
    config: &plugin_config::Model,
) -> Option<PluginAssignment> {
    let plugin_type: uptrakit_internal_wire::PluginType =
        serde_json::from_value(serde_json::Value::String(config.plugin_type.clone())).ok()?;

    let merged_config =
        crate::update_hooks::merge_config(&config.config, assignment.config_override.as_ref());

    Some(PluginAssignment {
        plugin_type,
        package_identifier: assignment.package_identifier.clone(),
        config: merged_config,
    })
}
