//! Query helper that loads software state data for MQTT `SoftwareStates` push messages.

use sea_orm::{ColumnTrait, Condition, EntityTrait, FromQueryResult, QueryFilter, QuerySelect};
use std::collections::HashMap;
use uptrakit_internal_wire::{
    MqttSoftwareStateHostEntry, MqttSoftwareStateItem, MqttSoftwareStatesPayload,
};
use uptrakit_shared_db::SoftwareDiscoveryState;
use uptrakit_shared_db::entity::{available_version, host, host_software_item, prelude::*, software_item};
use uuid::Uuid;

/// Lightweight projection used to bulk-load available version data.
#[derive(Debug, FromQueryResult)]
struct AvailableVersionRow {
    software_item_id: Uuid,
    version: Option<String>,
}

/// Lightweight projection used to bulk-load host-software-item link data.
#[derive(Debug, FromQueryResult)]
struct HostSoftwareItemRow {
    host_id: Uuid,
    software_item_id: Uuid,
    installed_version: Option<String>,
}

/// Load all software state data for a tenant and assemble a [`MqttSoftwareStatesPayload`].
///
/// This function executes four bulk queries (no N+1) and is safe to call on
/// every version-check result or update completion event.
///
/// # Errors
///
/// Returns a [`sea_orm::DbErr`] if any database query fails.
pub async fn load_software_states_for_tenant(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<MqttSoftwareStatesPayload, sea_orm::DbErr> {
    // 1. Load all active, non-deactivated software items for the tenant that are
    //    not in the Pending discovery state (i.e. NULL or Approved).
    let items = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::Enabled.eq(true))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .filter(
            Condition::any()
                .add(software_item::Column::DiscoveryState.is_null())
                .add(
                    software_item::Column::DiscoveryState
                        .eq(SoftwareDiscoveryState::Approved),
                ),
        )
        .all(db)
        .await?;

    // 2. Nothing to do — return early with an empty payload.
    if items.is_empty() {
        return Ok(MqttSoftwareStatesPayload {
            tenant_id,
            items: vec![],
        });
    }

    let item_ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();

    // 3. Bulk-load available_version rows for all items (one query).
    let available_version_rows: Vec<AvailableVersionRow> = AvailableVersion::find()
        .select_only()
        .column(available_version::Column::SoftwareItemId)
        .column(available_version::Column::Version)
        .filter(available_version::Column::SoftwareItemId.is_in(item_ids.clone()))
        .into_model::<AvailableVersionRow>()
        .all(db)
        .await?;

    // Map: software_item_id -> latest_version.
    let latest_versions: HashMap<Uuid, String> = available_version_rows
        .into_iter()
        .filter_map(|r| r.version.map(|v| (r.software_item_id, v)))
        .collect();

    // 4. Bulk-load host_software_item rows for all items (one query).
    let hsi_rows: Vec<HostSoftwareItemRow> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::HostId)
        .column(host_software_item::Column::SoftwareItemId)
        .column(host_software_item::Column::InstalledVersion)
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.clone()))
        .into_model::<HostSoftwareItemRow>()
        .all(db)
        .await?;

    // Collect distinct host_ids referenced in hsi_rows.
    let host_ids: Vec<Uuid> = {
        let mut seen = std::collections::HashSet::new();
        hsi_rows
            .iter()
            .filter(|r| seen.insert(r.host_id))
            .map(|r| r.host_id)
            .collect()
    };

    // 5. Bulk-load active host rows for those host_ids (one query).
    let active_hosts: HashMap<Uuid, host::Model> = if host_ids.is_empty() {
        HashMap::new()
    } else {
        Host::find()
            .filter(host::Column::Id.is_in(host_ids))
            .filter(host::Column::DeactivatedAt.is_null())
            .all(db)
            .await?
            .into_iter()
            .map(|h| (h.id, h))
            .collect()
    };

    // Index hsi rows by software_item_id for O(1) lookup during assembly.
    let mut hsi_by_item: HashMap<Uuid, Vec<&HostSoftwareItemRow>> = HashMap::new();
    for row in &hsi_rows {
        hsi_by_item.entry(row.software_item_id).or_default().push(row);
    }

    // 6. Assemble the payload.
    let mut result_items: Vec<MqttSoftwareStateItem> = Vec::with_capacity(items.len());

    for item in &items {
        let latest_version = latest_versions.get(&item.id).cloned();

        let host_entries: Vec<MqttSoftwareStateHostEntry> = hsi_by_item
            .get(&item.id)
            .map(|links| {
                links
                    .iter()
                    .filter_map(|link| {
                        let host = active_hosts.get(&link.host_id)?;
                        let update_available = match (
                            link.installed_version.as_deref(),
                            latest_version.as_deref(),
                        ) {
                            (Some(installed), Some(latest)) => installed != latest,
                            _ => false,
                        };
                        Some(MqttSoftwareStateHostEntry {
                            host_id: host.id,
                            hostname: host.hostname.clone(),
                            installed_version: link.installed_version.clone(),
                            latest_version: latest_version.clone(),
                            update_available,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Skip software items with no active host entries.
        if host_entries.is_empty() {
            continue;
        }

        result_items.push(MqttSoftwareStateItem {
            software_item_id: item.id,
            name: item.name.clone(),
            hosts: host_entries,
        });
    }

    Ok(MqttSoftwareStatesPayload {
        tenant_id,
        items: result_items,
    })
}
