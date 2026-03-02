//! Query helper that loads software state data for MQTT `SoftwareStates` push messages.

use sea_orm::{ColumnTrait, Condition, EntityTrait, FromQueryResult, QueryFilter, QuerySelect};
use std::collections::{HashMap, HashSet};
use uptrakit_internal_wire::{
    MqttHostPackageHostState, MqttSoftwareStateHostEntry, MqttSoftwareStateItem,
    MqttSoftwareStatesPayload,
};
use uptrakit_shared_db::entity::{
    host, host_package, host_package_update_history, host_software_item, prelude::*,
    software_item, update_history,
};
use uptrakit_shared_types::SoftwareDiscoveryState;
use uuid::Uuid;

/// Lightweight projection used to bulk-load host-software-item link data.
#[derive(Debug, FromQueryResult)]
struct HostSoftwareItemRow {
    host_id: Uuid,
    software_item_id: Uuid,
    installed_version: Option<String>,
    latest_version: Option<String>,
    latest_release_metadata: Option<serde_json::Value>,
}

/// Lightweight projection used to bulk-load active update records.
#[derive(Debug, FromQueryResult)]
struct ActiveUpdateRow {
    host_id: Uuid,
    software_item_id: Uuid,
}

/// Load all software state data for a tenant and assemble a [`MqttSoftwareStatesPayload`].
///
/// This function executes four bulk queries (no N+1) and is safe to call on
/// every version-check result, update-trigger, or update completion event.
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
                .add(software_item::Column::DiscoveryState.eq(SoftwareDiscoveryState::Approved)),
        )
        .all(db)
        .await?;

    // 2. Nothing to do — return early with an empty payload.
    if items.is_empty() {
        return Ok(MqttSoftwareStatesPayload {
            tenant_id,
            items: vec![],
            host_package_hosts: vec![],
        });
    }

    let item_ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();

    // 3. Bulk-load host_software_item rows (including per-host latest_version) for all items.
    let hsi_rows: Vec<HostSoftwareItemRow> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::HostId)
        .column(host_software_item::Column::SoftwareItemId)
        .column(host_software_item::Column::InstalledVersion)
        .column(host_software_item::Column::LatestVersion)
        .column(host_software_item::Column::LatestReleaseMetadata)
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.clone()))
        .into_model::<HostSoftwareItemRow>()
        .all(db)
        .await?;

    // Collect distinct host_ids referenced in hsi_rows.
    let host_ids: Vec<Uuid> = {
        let mut seen = HashSet::new();
        hsi_rows
            .iter()
            .filter(|r| seen.insert(r.host_id))
            .map(|r| r.host_id)
            .collect()
    };

    // 4. Bulk-load active host rows for those host_ids (one query).
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

    // 5. Bulk-load active update records (Pending or InProgress) for all items.
    //    Builds a HashSet<(host_id, software_item_id)> for O(1) lookup.
    let active_updates: HashSet<(Uuid, Uuid)> = UpdateHistory::find()
        .select_only()
        .column(update_history::Column::HostId)
        .column(update_history::Column::SoftwareItemId)
        .filter(update_history::Column::SoftwareItemId.is_in(item_ids.clone()))
        .filter(
            Condition::any()
                .add(
                    update_history::Column::Status
                        .eq(update_history::UpdateStatus::Pending),
                )
                .add(
                    update_history::Column::Status
                        .eq(update_history::UpdateStatus::InProgress),
                ),
        )
        .into_model::<ActiveUpdateRow>()
        .all(db)
        .await?
        .into_iter()
        .map(|r| (r.host_id, r.software_item_id))
        .collect();

    // Index hsi rows by software_item_id for O(1) lookup during assembly.
    let mut hsi_by_item: HashMap<Uuid, Vec<&HostSoftwareItemRow>> = HashMap::new();
    for row in &hsi_rows {
        hsi_by_item
            .entry(row.software_item_id)
            .or_default()
            .push(row);
    }

    // 6. Assemble the payload.
    let mut result_items: Vec<MqttSoftwareStateItem> = Vec::with_capacity(items.len());

    for item in &items {
        let host_entries: Vec<MqttSoftwareStateHostEntry> = hsi_by_item
            .get(&item.id)
            .map(|links| {
                links
                    .iter()
                    .filter_map(|link| {
                        let host = active_hosts.get(&link.host_id)?;
                        let update_available = match (
                            link.installed_version.as_deref(),
                            link.latest_version.as_deref(),
                        ) {
                            (Some(installed), Some(latest)) => installed != latest,
                            _ => false,
                        };
                        let update_in_progress =
                            active_updates.contains(&(link.host_id, link.software_item_id));
                        let (release_url, release_notes) =
                            extract_release_info(link.latest_release_metadata.as_ref());
                        Some(MqttSoftwareStateHostEntry {
                            host_id: host.id,
                            hostname: host.hostname.clone(),
                            installed_version: link.installed_version.clone(),
                            latest_version: link.latest_version.clone(),
                            update_available,
                            update_in_progress,
                            release_url,
                            release_notes,
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
        host_package_hosts: vec![],
    })
}

// ---------------------------------------------------------------------------
// Host package host states
// ---------------------------------------------------------------------------

/// Lightweight projection for bulk-loading host package rows.
#[derive(Debug, FromQueryResult)]
struct HostPackageRow {
    host_id: Uuid,
    installed_version: Option<String>,
    latest_version: Option<String>,
    update_category: String,
}

/// Lightweight projection for bulk-loading just the host_id from history rows.
#[derive(Debug, FromQueryResult)]
struct HistoryHostIdRow {
    host_id: Uuid,
}

/// Load per-host package state data for all hosts that have at least one
/// enabled, non-deactivated package under the given tenant.
///
/// Returns one [`MqttHostPackageHostState`] per qualifying host. Hosts with no
/// tracked packages are omitted from the result.
///
/// # Errors
///
/// Returns a [`sea_orm::DbErr`] if any database query fails.
pub async fn load_host_package_host_states_for_tenant(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<MqttHostPackageHostState>, sea_orm::DbErr> {
    // 1. Bulk-load all enabled, non-deactivated packages for this tenant.
    let packages: Vec<HostPackageRow> = HostPackage::find()
        .select_only()
        .column(host_package::Column::HostId)
        .column(host_package::Column::InstalledVersion)
        .column(host_package::Column::LatestVersion)
        .column(host_package::Column::UpdateCategory)
        .filter(host_package::Column::TenantId.eq(tenant_id))
        .filter(host_package::Column::Enabled.eq(true))
        .filter(host_package::Column::DeactivatedAt.is_null())
        .into_model::<HostPackageRow>()
        .all(db)
        .await?;

    if packages.is_empty() {
        return Ok(vec![]);
    }

    // 2. Collect distinct host_ids referenced.
    let host_ids: Vec<Uuid> = {
        let mut seen = HashSet::new();
        packages
            .iter()
            .filter(|p| seen.insert(p.host_id))
            .map(|p| p.host_id)
            .collect()
    };

    // 3. Bulk-load active host records for those host_ids.
    let active_hosts: HashMap<Uuid, host::Model> = Host::find()
        .filter(host::Column::Id.is_in(host_ids))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(db)
        .await?
        .into_iter()
        .map(|h| (h.id, h))
        .collect();

    // 4. Bulk-load host_package_update_history rows that are Pending or
    //    InProgress — used to detect which hosts currently have an update
    //    running. We collect the distinct host_ids only.
    let in_progress_host_ids: HashSet<Uuid> = HostPackageUpdateHistory::find()
        .select_only()
        .column(host_package_update_history::Column::HostId)
        .filter(host_package_update_history::Column::TenantId.eq(tenant_id))
        .filter(
            Condition::any()
                .add(host_package_update_history::Column::Status.eq("pending"))
                .add(host_package_update_history::Column::Status.eq("in_progress")),
        )
        .into_model::<HistoryHostIdRow>()
        .all(db)
        .await?
        .into_iter()
        .map(|r| r.host_id)
        .collect();

    // 5. Group packages by host_id and build result entries.
    let mut by_host: HashMap<Uuid, Vec<&HostPackageRow>> = HashMap::new();
    for pkg in &packages {
        by_host.entry(pkg.host_id).or_default().push(pkg);
    }

    let mut results: Vec<MqttHostPackageHostState> = Vec::with_capacity(by_host.len());
    for (host_id, pkgs) in by_host {
        // Skip hosts without an active host record (e.g. deactivated).
        let Some(host) = active_hosts.get(&host_id) else {
            continue;
        };

        let total_count = pkgs.len() as u32;
        let pending_count = pkgs
            .iter()
            .filter(|p| match (&p.installed_version, &p.latest_version) {
                (Some(installed), Some(latest)) => installed != latest,
                _ => false,
            })
            .count() as u32;
        let security_pending_count = pkgs
            .iter()
            .filter(|p| {
                matches!(
                    (&p.installed_version, &p.latest_version),
                    (Some(installed), Some(latest)) if installed != latest
                ) && p.update_category == "security"
            })
            .count() as u32;
        let update_in_progress = in_progress_host_ids.contains(&host_id);

        results.push(MqttHostPackageHostState {
            host_id,
            hostname: host.hostname.clone(),
            pending_count,
            security_pending_count,
            total_count,
            update_in_progress,
        });
    }

    Ok(results)
}

/// Extract `release_url` and `release_notes` from a `latest_release_metadata` JSON blob.
///
/// Returns `(None, None)` when `metadata` is `None` or the expected keys are absent.
fn extract_release_info(
    metadata: Option<&serde_json::Value>,
) -> (Option<String>, Option<String>) {
    let Some(meta) = metadata else {
        return (None, None);
    };
    let url = meta
        .get("release_url")
        .and_then(|v| v.as_str())
        .map(String::from);
    let notes = meta
        .get("release_notes")
        .and_then(|v| v.as_str())
        .map(String::from);
    (url, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_release_info_none_input() {
        assert_eq!(extract_release_info(None), (None, None));
    }

    #[test]
    fn extract_release_info_both_fields_present() {
        let meta = serde_json::json!({
            "release_url": "https://github.com/owner/repo/releases/tag/v1.3.0",
            "release_notes": "## What's new\n- Feature A"
        });
        let (url, notes) = extract_release_info(Some(&meta));
        assert_eq!(
            url.as_deref(),
            Some("https://github.com/owner/repo/releases/tag/v1.3.0")
        );
        assert_eq!(notes.as_deref(), Some("## What's new\n- Feature A"));
    }

    #[test]
    fn extract_release_info_missing_fields() {
        let meta = serde_json::json!({ "tag": "v1.3.0" });
        assert_eq!(extract_release_info(Some(&meta)), (None, None));
    }

    #[test]
    fn extract_release_info_only_url() {
        let meta = serde_json::json!({ "release_url": "https://example.com" });
        let (url, notes) = extract_release_info(Some(&meta));
        assert_eq!(url.as_deref(), Some("https://example.com"));
        assert!(notes.is_none());
    }

    #[test]
    fn extract_release_info_non_string_values_ignored() {
        let meta = serde_json::json!({ "release_url": 42, "release_notes": true });
        assert_eq!(extract_release_info(Some(&meta)), (None, None));
    }

    // -------------------------------------------------------------------------
    // security_pending_count computation
    // -------------------------------------------------------------------------

    fn make_pkg(installed: Option<&str>, latest: Option<&str>, category: &str) -> HostPackageRow {
        HostPackageRow {
            host_id: Uuid::nil(),
            installed_version: installed.map(String::from),
            latest_version: latest.map(String::from),
            update_category: category.to_string(),
        }
    }

    #[test]
    fn security_pending_count_only_security_outdated() {
        // 3 packages: one security-outdated, one regular-outdated, one up-to-date security
        let pkgs = vec![
            make_pkg(Some("1.0"), Some("2.0"), "security"),
            make_pkg(Some("1.0"), Some("2.0"), "bugfix"),
            make_pkg(Some("3.0"), Some("3.0"), "security"),
        ];
        let security_pending_count = pkgs
            .iter()
            .filter(|p| {
                matches!(
                    (&p.installed_version, &p.latest_version),
                    (Some(installed), Some(latest)) if installed != latest
                ) && p.update_category == "security"
            })
            .count() as u32;
        assert_eq!(security_pending_count, 1);
    }

    #[test]
    fn security_pending_count_zero_when_no_security_packages() {
        let pkgs = vec![
            make_pkg(Some("1.0"), Some("2.0"), "bugfix"),
            make_pkg(Some("1.0"), Some("1.0"), "regular"),
        ];
        let security_pending_count = pkgs
            .iter()
            .filter(|p| {
                matches!(
                    (&p.installed_version, &p.latest_version),
                    (Some(installed), Some(latest)) if installed != latest
                ) && p.update_category == "security"
            })
            .count() as u32;
        assert_eq!(security_pending_count, 0);
    }

    #[test]
    fn security_pending_count_ignores_missing_versions() {
        let pkgs = vec![
            make_pkg(None, Some("2.0"), "security"),
            make_pkg(Some("1.0"), None, "security"),
        ];
        let security_pending_count = pkgs
            .iter()
            .filter(|p| {
                matches!(
                    (&p.installed_version, &p.latest_version),
                    (Some(installed), Some(latest)) if installed != latest
                ) && p.update_category == "security"
            })
            .count() as u32;
        assert_eq!(security_pending_count, 0);
    }
}
