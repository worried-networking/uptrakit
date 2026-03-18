//! Controller-side `fetch_releases` orchestration.
//!
//! When a plugin assignment has `execution_site == "controller"` (or `"auto"` with a plugin
//! that declares [`PluginCapability::ControllerSideFetchReleases`]), version checks run on the
//! controller rather than being dispatched to the agent. This module contains the job type,
//! the site-selection predicate, and the async job runner.

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, prelude::Expr};
use time::OffsetDateTime;
use uptrakit_plugin_infrastructure_registry::{
    ControllerRuntime, PluginCapability, PluginType, get_descriptor,
};
use uptrakit_shared_db::entity::{host_software_item, software_item};
use uptrakit_web_api_types::events::AdminEvent;
use uuid::Uuid;

/// Describes a single controller-side `fetch_releases` job.
pub(super) struct ControllerFetchJob {
    pub(super) plugin_type: PluginType,
    pub(super) package_identifier: String,
    pub(super) merged_config: serde_json::Value,
    /// All `(host_id, software_item_id)` pairs that share this plugin+package.
    pub(super) targets: Vec<(Uuid, Uuid)>,
}

/// Returns `true` if a `fetch_releases` assignment should run on the controller.
///
/// - `execution_site == "controller"` -> always controller.
/// - `execution_site == "agent"` -> always agent.
/// - `execution_site == "auto"` -> controller when the plugin declares
///   [`PluginCapability::ControllerSideFetchReleases`].
pub(super) fn is_controller_fetch_site(
    execution_site: &str,
    plugin_type: &PluginType,
    _config: &serde_json::Value,
) -> bool {
    match execution_site {
        "controller" => true,
        "agent" => false,
        _ => {
            // "auto" -- check static capability via descriptor (no instantiation needed)
            get_descriptor(plugin_type.as_str())
                .map(|desc| {
                    desc.capabilities
                        .contains(&PluginCapability::ControllerSideFetchReleases)
                })
                .unwrap_or(false)
        }
    }
}

/// Execute controller-side `fetch_releases` for a batch of jobs.
///
/// Groups by `(plugin_type, package_identifier, config)` deduplication has
/// already been applied by the caller -- each job represents one distinct API
/// call. Updates `host_software_item.latest_version`,
/// `latest_version_fetched_at`, and `software_item.last_checked_at` for all
/// successful fetches. Pushes MQTT software states after updating.
///
/// Returns the number of jobs for which `fetch_releases` succeeded.
pub(super) async fn run_controller_fetch_jobs(
    db: &sea_orm::DatabaseConnection,
    notification_service: &crate::notification_service::NotificationService,
    event_broadcaster: &crate::event_broadcaster::EventBroadcaster,
    tenant_id: Uuid,
    jobs: Vec<ControllerFetchJob>,
) -> u32 {
    if jobs.is_empty() {
        return 0;
    }

    let controller_runtime: Arc<dyn uptrakit_plugin_infrastructure_registry::HostRuntime> =
        Arc::new(ControllerRuntime::new(
            uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        ));
    let now = OffsetDateTime::now_utc();
    let mut succeeded = 0u32;
    let mut updated_item_ids: HashSet<Uuid> = HashSet::new();
    let mut completed_pairs: Vec<(Uuid, Uuid)> = Vec::new();

    for job in &jobs {
        let type_str = job.plugin_type.as_str();

        let desc = match get_descriptor(type_str) {
            Some(d) => d,
            None => {
                tracing::warn!(
                    plugin_type = type_str,
                    package = %job.package_identifier,
                    "controller-side fetch: unknown plugin type"
                );
                continue;
            }
        };

        let slot = match desc.roles.release_fetcher.as_ref() {
            Some(s) => s,
            None => {
                tracing::warn!(
                    plugin_type = type_str,
                    "plugin does not support release_fetcher role; skipping"
                );
                continue;
            }
        };

        let fetcher = match (slot.create)(&job.merged_config, controller_runtime.clone()) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(
                    plugin_type = type_str,
                    package = %job.package_identifier,
                    error = %e,
                    "controller-side fetch: failed to create release fetcher"
                );
                continue;
            }
        };

        let releases = match fetcher.fetch_releases(&job.package_identifier).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    plugin_type = type_str,
                    package = %job.package_identifier,
                    error = %e,
                    "controller-side fetch: fetch_releases failed"
                );
                continue;
            }
        };

        let latest = releases
            .iter()
            .find(|r| !r.is_prerelease)
            .or(releases.first());
        let Some(latest) = latest else {
            tracing::debug!(
                plugin_type = type_str,
                package = %job.package_identifier,
                "controller-side fetch: no releases returned"
            );
            continue;
        };

        let latest_version_str = latest.version.to_string();
        let release_metadata = serde_json::to_value(latest).unwrap_or(serde_json::Value::Null);

        tracing::debug!(
            plugin_type = type_str,
            package = %job.package_identifier,
            latest_version = %latest_version_str,
            host_count = job.targets.len(),
            "controller-side fetch: succeeded"
        );

        let category_str = latest.category.clone().unwrap_or_default().to_string();

        for (host_id, software_item_id) in &job.targets {
            match host_software_item::Entity::update_many()
                .col_expr(
                    host_software_item::Column::LatestVersion,
                    Expr::value(Some(latest_version_str.clone())),
                )
                .col_expr(
                    host_software_item::Column::LatestVersionFetchedAt,
                    Expr::value(Some(now)),
                )
                .col_expr(
                    host_software_item::Column::LatestReleaseMetadata,
                    Expr::value(Some(release_metadata.clone())),
                )
                .col_expr(
                    host_software_item::Column::UpdateCategory,
                    Expr::value(category_str.clone()),
                )
                .filter(host_software_item::Column::HostId.eq(*host_id))
                .filter(host_software_item::Column::SoftwareItemId.eq(*software_item_id))
                .exec(db)
                .await
            {
                Err(e) => {
                    tracing::warn!(
                        host_id = %host_id,
                        software_item_id = %software_item_id,
                        error = %e,
                        "controller-side fetch: failed to update host_software_item"
                    );
                }
                Ok(_) => {
                    updated_item_ids.insert(*software_item_id);
                    completed_pairs.push((*host_id, *software_item_id));
                }
            }
        }
        succeeded += 1;
    }

    if !updated_item_ids.is_empty() {
        // Batch-update software_item.last_checked_at for all successfully fetched items.
        let item_ids: Vec<Uuid> = updated_item_ids.into_iter().collect();
        if let Err(e) = software_item::Entity::update_many()
            .filter(software_item::Column::Id.is_in(item_ids))
            .col_expr(software_item::Column::LastCheckedAt, Expr::value(now))
            .exec(db)
            .await
        {
            tracing::warn!(error = %e, "controller-side fetch: failed to update last_checked_at");
        }

        // Push updated software states to MQTT services.
        notification_service
            .push_software_states_for_tenant(db, tenant_id)
            .await;

        // Emit AdminEvent::VersionCheckCompleted for each updated pair so the
        // /software page SSE subscribers refresh when controller-side fetches complete.
        for (host_id, software_item_id) in completed_pairs {
            event_broadcaster
                .send(
                    tenant_id,
                    AdminEvent::VersionCheckCompleted {
                        host_id,
                        software_item_id,
                    },
                )
                .await;
        }
    }

    succeeded
}
