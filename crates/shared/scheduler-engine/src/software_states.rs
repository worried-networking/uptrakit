//! Query helper that loads software state data for MQTT `SoftwareStates` push messages.
//!
//! This is the **single canonical implementation** shared by all code paths:
//! - The web-API tier re-exports [`load_software_states_for_tenant`] via
//!   `uptrakit_web_api_queries::queries::mqtt_software_states`.
//! - The external-scheduler path calls it directly from this module.

use sea_orm::{
    ColumnTrait, Condition, EntityTrait, FromQueryResult, JoinType, QueryFilter, QuerySelect,
    RelationTrait as _,
};
use std::collections::{HashMap, HashSet};
use uptrakit_internal_wire::{
    MqttHostMetadata, MqttHostSummary, MqttSoftwareStateHostEntry, MqttSoftwareStateItem,
    MqttSoftwareStatesPayload,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_tag, host_tag_assignment, prelude::*, service, service_host,
    software_item, update_history,
};
use uptrakit_shared_types::ServiceStatus;
use uuid::Uuid;

/// Lightweight projection used to bulk-load host-software-item link data.
#[derive(Debug, FromQueryResult)]
struct HostSoftwareItemRow {
    host_id: Uuid,
    software_item_id: Uuid,
    installed_version: Option<String>,
    installed_version_detected_at: Option<time::OffsetDateTime>,
    latest_version: Option<String>,
    latest_release_metadata: Option<serde_json::Value>,
    update_category: String,
}

/// Lightweight projection used to bulk-load active update records.
#[derive(Debug, FromQueryResult)]
struct ActiveUpdateRow {
    host_id: Uuid,
    software_item_id: Uuid,
}

/// Load all software state data for a tenant and assemble a [`MqttSoftwareStatesPayload`].
///
/// Only **featured** software items are included as individual MQTT entities in
/// `payload.items`. Non-featured items are aggregated into per-host summaries in
/// `payload.host_summaries`.
///
/// This function executes five bulk queries (no N+1) and is safe to call on
/// every version-check result or update completion event.
///
/// The web-API tier re-exports this function from
/// `uptrakit_web_api_queries::queries::mqtt_software_states`.
///
/// # Errors
///
/// Returns a [`sea_orm::DbErr`] if any database query fails.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn load_software_states_for_tenant(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<MqttSoftwareStatesPayload, sea_orm::DbErr> {
    // 1. Load all active, non-deactivated software items for the tenant.
    let items = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await?;

    // 2. Nothing to do — return early with an empty payload.
    if items.is_empty() {
        return Ok(MqttSoftwareStatesPayload {
            tenant_id,
            items: vec![],
            host_summaries: vec![],
            hosts: vec![],
        });
    }

    let item_ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();

    // 3. Bulk-load host_software_item rows (including per-host latest_version) for all items.
    //    Filter out deactivated rows.
    let hsi_rows: Vec<HostSoftwareItemRow> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::HostId)
        .column(host_software_item::Column::SoftwareItemId)
        .column(host_software_item::Column::InstalledVersion)
        .column(host_software_item::Column::InstalledVersionDetectedAt)
        .column(host_software_item::Column::LatestVersion)
        .column(host_software_item::Column::LatestReleaseMetadata)
        .column(host_software_item::Column::UpdateCategory)
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.clone()))
        .filter(host_software_item::Column::DeactivatedAt.is_null())
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

    // 5. Bulk-load active update records (Queued, Pending, or InProgress) for all items.
    //    `Queued` is included because it represents a committed intent to update —
    //    the host IS going to update even though execution hasn't started yet.
    //    Builds a HashSet<(host_id, software_item_id)> for O(1) lookup.
    let active_updates: HashSet<(Uuid, Uuid)> = UpdateHistory::find()
        .select_only()
        .column(update_history::Column::HostId)
        .column(update_history::Column::SoftwareItemId)
        .filter(update_history::Column::SoftwareItemId.is_in(item_ids.clone()))
        .filter(
            Condition::any()
                .add(update_history::Column::Status.eq(update_history::UpdateStatus::Queued))
                .add(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
                .add(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress)),
        )
        .into_model::<ActiveUpdateRow>()
        .all(db)
        .await?
        .into_iter()
        .map(|r| (r.host_id, r.software_item_id))
        .collect();

    // Build a set of featured item IDs for distinguishing featured vs unfeatured.
    let featured_item_ids: HashSet<Uuid> =
        items.iter().filter(|i| i.featured).map(|i| i.id).collect();

    // Index hsi rows by software_item_id for O(1) lookup during assembly.
    let mut hsi_by_item: HashMap<Uuid, Vec<&HostSoftwareItemRow>> = HashMap::new();
    for row in &hsi_rows {
        hsi_by_item
            .entry(row.software_item_id)
            .or_default()
            .push(row);
    }

    // 6. Assemble the featured items payload (individual MQTT entities).
    let mut result_items: Vec<MqttSoftwareStateItem> = Vec::with_capacity(items.len());

    for item in &items {
        // Only featured items get individual MQTT entities.
        if !item.featured {
            continue;
        }

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
                        let (release_url, release_notes, release_date) =
                            extract_release_info(link.latest_release_metadata.as_ref());
                        let last_checked_at = link.installed_version_detected_at.map(|dt| {
                            dt.format(&time::format_description::well_known::Rfc3339)
                                .unwrap_or_default()
                        });
                        Some(MqttSoftwareStateHostEntry {
                            host_id: host.id,
                            hostname: host.hostname.clone(),
                            friendly_name: host.friendly_name.clone(),
                            installed_version: link.installed_version.clone(),
                            latest_version: link.latest_version.clone(),
                            update_available,
                            update_in_progress,
                            release_url,
                            release_notes,
                            update_category: Some(link.update_category.clone()),
                            release_date,
                            last_checked_at,
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
            icon_url: item.icon_url.clone(),
            hosts: host_entries,
        });
    }

    // 7. Build per-host summaries for unfeatured items.
    //    Group all unfeatured hsi_rows by host_id and compute aggregates.
    let mut unfeatured_by_host: HashMap<Uuid, Vec<&HostSoftwareItemRow>> = HashMap::new();
    for row in &hsi_rows {
        if !featured_item_ids.contains(&row.software_item_id) {
            unfeatured_by_host.entry(row.host_id).or_default().push(row);
        }
    }

    // Collect active update host_ids for unfeatured items to detect batch updates.
    let unfeatured_in_progress_hosts: HashSet<Uuid> = active_updates
        .iter()
        .filter(|(_, si_id)| !featured_item_ids.contains(si_id))
        .map(|(h_id, _)| *h_id)
        .collect();

    let mut host_summaries: Vec<MqttHostSummary> = Vec::with_capacity(unfeatured_by_host.len());
    for (host_id, rows) in unfeatured_by_host {
        let Some(host) = active_hosts.get(&host_id) else {
            continue;
        };

        let is_outdated = |r: &&&HostSoftwareItemRow| {
            matches!(
                (&r.installed_version, &r.latest_version),
                (Some(installed), Some(latest)) if installed != latest
            )
        };
        let total_count = rows.len() as u32;
        let pending_count = rows.iter().filter(is_outdated).count() as u32;
        let security_pending_count = rows
            .iter()
            .filter(|r| is_outdated(r) && r.update_category == "security")
            .count() as u32;
        let bugfix_count = rows
            .iter()
            .filter(|r| is_outdated(r) && r.update_category == "bugfix")
            .count() as u32;
        let feature_count = rows
            .iter()
            .filter(|r| is_outdated(r) && r.update_category == "feature")
            .count() as u32;
        let update_in_progress = unfeatured_in_progress_hosts.contains(&host_id);

        host_summaries.push(MqttHostSummary {
            host_id,
            hostname: host.hostname.clone(),
            friendly_name: host.friendly_name.clone(),
            pending_count,
            security_pending_count,
            total_count,
            update_in_progress,
            bugfix_count,
            feature_count,
        });
    }

    // 8. Build MqttHostMetadata for all active hosts.
    let host_metadata = build_host_metadata(db, &active_hosts).await?;

    Ok(MqttSoftwareStatesPayload {
        tenant_id,
        items: result_items,
        host_summaries,
        hosts: host_metadata,
    })
}

// ---------------------------------------------------------------------------
// Host metadata
// ---------------------------------------------------------------------------

/// Projection for host tags.
#[derive(Debug, FromQueryResult)]
struct HostTagRow {
    host_id: Uuid,
    name: String,
}

/// Projection for agent info linked to a host.
#[derive(Debug, FromQueryResult)]
struct AgentInfoRow {
    host_id: Uuid,
    client_version: Option<String>,
    last_seen_at: Option<time::OffsetDateTime>,
}

/// Build `Vec<MqttHostMetadata>` for all hosts in `active_hosts`.
///
/// Performs two additional bulk queries:
/// 1. `host_tag_assignments JOIN host_tags` to get tag names per host.
/// 2. `service_hosts JOIN services` to get agent version and last_seen_at.
async fn build_host_metadata(
    db: &sea_orm::DatabaseConnection,
    active_hosts: &HashMap<Uuid, host::Model>,
) -> Result<Vec<MqttHostMetadata>, sea_orm::DbErr> {
    if active_hosts.is_empty() {
        return Ok(vec![]);
    }

    let host_ids: Vec<Uuid> = active_hosts.keys().copied().collect();

    // 1. Load tags for all hosts in a single join query.
    let tag_rows: Vec<HostTagRow> = HostTagAssignment::find()
        .select_only()
        .column(host_tag_assignment::Column::HostId)
        .column_as(host_tag::Column::Name, "name")
        .join(
            JoinType::InnerJoin,
            host_tag_assignment::Relation::HostTag.def(),
        )
        .filter(host_tag_assignment::Column::HostId.is_in(host_ids.clone()))
        // Exclude deactivated tags.
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .into_model::<HostTagRow>()
        .all(db)
        .await?;

    // Index tags by host_id.
    let mut tags_by_host: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in tag_rows {
        tags_by_host.entry(row.host_id).or_default().push(row.name);
    }

    // 2. Load agent info (client_version, last_seen_at) for all hosts.
    //    Uses service_hosts JOIN services, picking approved, non-deactivated agents.
    let agent_rows: Vec<AgentInfoRow> = ServiceHost::find()
        .select_only()
        .column(service_host::Column::HostId)
        .column_as(service::Column::ClientVersion, "client_version")
        .column_as(service::Column::LastSeenAt, "last_seen_at")
        .join(JoinType::InnerJoin, service_host::Relation::Service.def())
        .filter(service_host::Column::HostId.is_in(host_ids))
        .filter(service::Column::Status.eq(ServiceStatus::Approved))
        .filter(service::Column::DeactivatedAt.is_null())
        .into_model::<AgentInfoRow>()
        .all(db)
        .await?;

    // Index agent info by host_id — take the most-recently-seen agent per host.
    let mut agent_by_host: HashMap<Uuid, AgentInfoRow> = HashMap::new();
    for row in agent_rows {
        let entry = agent_by_host
            .entry(row.host_id)
            .or_insert_with(|| AgentInfoRow {
                host_id: row.host_id,
                client_version: None,
                last_seen_at: None,
            });
        // Prefer the agent with the latest last_seen_at.
        if row.last_seen_at > entry.last_seen_at {
            entry.client_version = row.client_version;
            entry.last_seen_at = row.last_seen_at;
        }
    }

    // 3. Build result.
    let mut metadata: Vec<MqttHostMetadata> = Vec::with_capacity(active_hosts.len());
    for host in active_hosts.values() {
        let tags = tags_by_host.remove(&host.id).unwrap_or_default();
        let agent = agent_by_host.remove(&host.id);
        let agent_last_seen_at = agent.as_ref().and_then(|a| {
            a.last_seen_at.map(|dt| {
                dt.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default()
            })
        });
        let agent_version = agent.and_then(|a| a.client_version);

        let mut m =
            MqttHostMetadata::new(host.id, host.hostname.clone(), host.friendly_name.clone());
        m.os_type = host.os_type.clone();
        m.os_version = host.os_version.clone();
        m.architecture = host.architecture.clone();
        m.tags = tags;
        m.agent_version = agent_version;
        m.agent_last_seen_at = agent_last_seen_at;
        metadata.push(m);
    }

    Ok(metadata)
}

/// Extract `release_url`, `release_notes`, and `release_date` from a
/// `latest_release_metadata` JSON blob.
///
/// Returns `(None, None, None)` when `metadata` is `None` or the expected
/// keys are absent.
fn extract_release_info(
    metadata: Option<&serde_json::Value>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(meta) = metadata else {
        return (None, None, None);
    };
    let url = meta
        .get("release_url")
        .and_then(|v| v.as_str())
        .map(String::from);
    let notes = meta
        .get("release_notes")
        .and_then(|v| v.as_str())
        .map(String::from);
    // `published_at` may be a full ISO 8601 datetime or a date-only string.
    // We take the first 10 chars (YYYY-MM-DD) for the MQTT date field.
    let release_date = meta
        .get("published_at")
        .and_then(|v| v.as_str())
        .map(|s| s.chars().take(10).collect::<String>());
    (url, notes, release_date)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_release_info_none_input() {
        assert_eq!(extract_release_info(None), (None, None, None));
    }

    #[test]
    fn extract_release_info_both_fields_present() {
        let meta = serde_json::json!({
            "release_url": "https://github.com/owner/repo/releases/tag/v1.3.0",
            "release_notes": "## What's new\n- Feature A"
        });
        let (url, notes, date) = extract_release_info(Some(&meta));
        assert_eq!(
            url.as_deref(),
            Some("https://github.com/owner/repo/releases/tag/v1.3.0")
        );
        assert_eq!(notes.as_deref(), Some("## What's new\n- Feature A"));
        assert!(date.is_none());
    }

    #[test]
    fn extract_release_info_missing_fields() {
        let meta = serde_json::json!({ "tag": "v1.3.0" });
        assert_eq!(extract_release_info(Some(&meta)), (None, None, None));
    }

    #[test]
    fn extract_release_info_only_url() {
        let meta = serde_json::json!({ "release_url": "https://example.com" });
        let (url, notes, date) = extract_release_info(Some(&meta));
        assert_eq!(url.as_deref(), Some("https://example.com"));
        assert!(notes.is_none());
        assert!(date.is_none());
    }

    #[test]
    fn extract_release_info_non_string_values_ignored() {
        let meta = serde_json::json!({ "release_url": 42, "release_notes": true });
        assert_eq!(extract_release_info(Some(&meta)), (None, None, None));
    }

    #[test]
    fn extract_release_info_published_at_datetime() {
        let meta = serde_json::json!({
            "release_url": "https://example.com",
            "published_at": "2025-01-15T10:00:00Z"
        });
        let (url, _notes, date) = extract_release_info(Some(&meta));
        assert_eq!(url.as_deref(), Some("https://example.com"));
        assert_eq!(date.as_deref(), Some("2025-01-15"));
    }

    #[test]
    fn extract_release_info_published_at_date_only() {
        let meta = serde_json::json!({ "published_at": "2025-06-30" });
        let (_url, _notes, date) = extract_release_info(Some(&meta));
        assert_eq!(date.as_deref(), Some("2025-06-30"));
    }

    // -------------------------------------------------------------------------
    // update_in_progress from active_updates
    // -------------------------------------------------------------------------

    #[test]
    fn active_updates_set_drives_update_in_progress() {
        let host_id = Uuid::nil();
        let si_id = Uuid::now_v7();
        let active_updates: HashSet<(Uuid, Uuid)> = [(host_id, si_id)].into();

        assert!(active_updates.contains(&(host_id, si_id)));
        assert!(!active_updates.contains(&(host_id, Uuid::now_v7())));
    }

    // -------------------------------------------------------------------------
    // security_pending_count computation
    // -------------------------------------------------------------------------

    fn make_row(
        installed: Option<&str>,
        latest: Option<&str>,
        category: &str,
    ) -> HostSoftwareItemRow {
        HostSoftwareItemRow {
            host_id: Uuid::nil(),
            software_item_id: Uuid::nil(),
            installed_version: installed.map(String::from),
            installed_version_detected_at: None,
            latest_version: latest.map(String::from),
            latest_release_metadata: None,
            update_category: category.to_string(),
        }
    }

    #[test]
    fn security_pending_count_only_security_outdated() {
        let rows = [
            make_row(Some("1.0"), Some("2.0"), "security"),
            make_row(Some("1.0"), Some("2.0"), "bugfix"),
            make_row(Some("3.0"), Some("3.0"), "security"),
        ];
        let security_pending_count = rows
            .iter()
            .filter(|r| {
                matches!(
                    (&r.installed_version, &r.latest_version),
                    (Some(installed), Some(latest)) if installed != latest
                ) && r.update_category == "security"
            })
            .count() as u32;
        assert_eq!(security_pending_count, 1);
    }

    #[test]
    fn security_pending_count_zero_when_no_security_items() {
        let rows = [
            make_row(Some("1.0"), Some("2.0"), "bugfix"),
            make_row(Some("1.0"), Some("1.0"), "regular"),
        ];
        let security_pending_count = rows
            .iter()
            .filter(|r| {
                matches!(
                    (&r.installed_version, &r.latest_version),
                    (Some(installed), Some(latest)) if installed != latest
                ) && r.update_category == "security"
            })
            .count() as u32;
        assert_eq!(security_pending_count, 0);
    }

    #[test]
    fn security_pending_count_ignores_missing_versions() {
        let rows = [
            make_row(None, Some("2.0"), "security"),
            make_row(Some("1.0"), None, "security"),
        ];
        let security_pending_count = rows
            .iter()
            .filter(|r| {
                matches!(
                    (&r.installed_version, &r.latest_version),
                    (Some(installed), Some(latest)) if installed != latest
                ) && r.update_category == "security"
            })
            .count() as u32;
        assert_eq!(security_pending_count, 0);
    }
}
