//! Update delivery and ownership validation helpers.
//!
//! Contains `validate_update_ownership`, `load_linked_host_ids`,
//! `upsert_available_version`, and `deliver_pending_updates` extracted from the
//! unified handler module.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use rootcause::prelude::*;
use uptrakit_internal_wire::{ControllerMessage, ExecuteUpdatePayload, OutgoingSeq, PluginType};
use uptrakit_shared_db::entity::{
    available_version, host, host_software_item, plugin_config, service_host, software_item,
    update_history,
};

use super::{HandlerError, HandlerResult};
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
// upsert_available_version
// ---------------------------------------------------------------------------

/// Upsert an `available_version` record for a software item.
///
/// If an existing record with the same version already exists for this software
/// item, its `updated_at` timestamp is refreshed. Otherwise, old records for
/// this software item are deleted and a new one is inserted.
pub(super) async fn upsert_available_version(
    db: &sea_orm::DatabaseConnection,
    software_item_id: uuid::Uuid,
    version: &str,
    now: time::OffsetDateTime,
) {
    // Check if a record with this version already exists.
    let existing = available_version::Entity::find()
        .filter(available_version::Column::SoftwareItemId.eq(software_item_id))
        .filter(available_version::Column::Version.eq(version))
        .one(db)
        .await;

    match existing {
        Ok(Some(record)) => {
            // Version already recorded -- just refresh the timestamp.
            let mut active: available_version::ActiveModel = record.into();
            active.updated_at = Set(now);
            if let Err(e) = active.update(db).await {
                tracing::warn!(
                    error = %e,
                    software_item_id = %software_item_id,
                    version,
                    "failed to update available_version timestamp"
                );
            }
        }
        Ok(None) => {
            // Delete any previous available_version records for this item
            // and insert the new one.
            if let Err(e) = available_version::Entity::delete_many()
                .filter(available_version::Column::SoftwareItemId.eq(software_item_id))
                .exec(db)
                .await
            {
                tracing::warn!(
                    error = %e,
                    software_item_id = %software_item_id,
                    "failed to delete old available_version records"
                );
            }

            let record = available_version::ActiveModel {
                id: Set(uuid::Uuid::now_v7()),
                software_item_id: Set(software_item_id),
                version: Set(Some(version.to_string())),
                release_date: Set(None),
                release_notes: Set(None),
                extra: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            };
            if let Err(e) = available_version::Entity::insert(record).exec(db).await {
                tracing::warn!(
                    error = %e,
                    software_item_id = %software_item_id,
                    version,
                    "failed to insert available_version"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                software_item_id = %software_item_id,
                "failed to query available_version"
            );
        }
    }
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

        // Load per-host plugin info from the host_software_item link.
        let link = match host_software_item::Entity::find_by_id((update_record.host_id, item.id))
            .one(state.db())
            .await
        {
            Ok(Some(l)) => l,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    host_id = %update_record.host_id,
                    software_item_id = %item.id,
                    "host-software-item link not found, skipping pending update"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to load host-software-item link for pending update"
                );
                continue;
            }
        };

        let plugin_cfg = match plugin_config::Entity::find_by_id(link.plugin_config_id)
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .one(state.db())
            .await
        {
            Ok(Some(c)) => c,
            Ok(None) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    plugin_config_id = %link.plugin_config_id,
                    "plugin config not found or deactivated, skipping pending update"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load plugin config for pending update");
                continue;
            }
        };

        let plugin_type: PluginType = match serde_json::from_value(serde_json::Value::String(
            plugin_cfg.plugin_type.clone(),
        )) {
            Ok(pt) => pt,
            Err(_) => {
                tracing::warn!(
                    update_id = %update_record.id,
                    plugin_type = %plugin_cfg.plugin_type,
                    "unknown plugin type, skipping pending update"
                );
                continue;
            }
        };

        let resolved_hooks =
            crate::update_hooks::resolve_hooks(&plugin_cfg.config, link.config_override.as_ref());
        let merged_config =
            crate::update_hooks::merge_config(&plugin_cfg.config, link.config_override.as_ref());

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
            package_identifier: link.package_identifier.clone(),
            to_version: update_record.to_version.clone(),
            plugin_type,
            plugin_config: merged_config,
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
