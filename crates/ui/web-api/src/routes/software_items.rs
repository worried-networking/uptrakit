use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{
    CanCreateSoftware, CanDeleteSoftware, CanTriggerChecks, CanTriggerUpdates, CanUpdateSoftware,
    CanViewSoftware,
};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::plugin_configs::find_raw_active_config;
use crate::queries::software_items::{self as item_queries, SoftwareItemQueryError};
use crate::queries::update_types::ActorType;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, RelationTrait, Set, prelude::Expr,
};
use std::collections::HashMap;
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_command::NoopCommandExecutor;
use uptrakit_plugin_infrastructure_registry::{
    CommandExecutor, PluginCapability, PluginRegistry, PluginType,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, service,
    service_host, software_item,
};
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, ListSoftwareItemsParams,
    SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse,
    TriggerUpdateRequest, TriggerUpdateResponse, TriggerUpdateStatus, TriggerVersionCheckResponse,
    UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
};

// --- Error mapping helper ---

fn query_error_to_response(report: rootcause::Report<SoftwareItemQueryError>) -> Response {
    match report.current_context() {
        SoftwareItemQueryError::NotFound => {
            error_response(StatusCode::NOT_FOUND, "Software item not found")
        }
        SoftwareItemQueryError::EmptyName => {
            error_response(StatusCode::BAD_REQUEST, "name must not be empty")
        }
        SoftwareItemQueryError::PluginConfigNotFound => error_response(
            StatusCode::BAD_REQUEST,
            "plugin_config_id does not reference an active plugin config",
        ),
        SoftwareItemQueryError::DuplicateItem => error_response(
            StatusCode::CONFLICT,
            "A software item with this name already exists",
        ),
        SoftwareItemQueryError::DuplicateHostAssignment => error_response(
            StatusCode::CONFLICT,
            "This host already has an assignment for the given plugin config and package identifier",
        ),
        SoftwareItemQueryError::HostNotFound(id) => error_response(
            StatusCode::BAD_REQUEST,
            format!("Host {id} not found or deactivated"),
        ),
        SoftwareItemQueryError::InvalidPackageIdentifier(msg)
        | SoftwareItemQueryError::InvalidConfigOverride(msg)
        | SoftwareItemQueryError::InvalidInlinePluginConfig(msg)
        | SoftwareItemQueryError::InvalidExecutionSite(msg) => {
            error_response(StatusCode::BAD_REQUEST, msg.clone())
        }
        SoftwareItemQueryError::Db(_) => {
            tracing::error!("Database error in software items: {report}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// --- Controller-side fetch_releases helpers ---

/// Describes a single controller-side `fetch_releases` job.
struct ControllerFetchJob {
    plugin_type: PluginType,
    package_identifier: String,
    merged_config: serde_json::Value,
    /// All `(host_id, software_item_id)` pairs that share this plugin+package.
    targets: Vec<(Uuid, Uuid)>,
}

/// Returns `true` if a `fetch_releases` assignment should run on the controller.
///
/// - `execution_site == "controller"` → always controller.
/// - `execution_site == "agent"` → always agent.
/// - `execution_site == "auto"` → controller when the plugin declares
///   [`PluginCapability::ControllerSideFetchReleases`].
fn is_controller_fetch_site(
    execution_site: &str,
    plugin_type: &PluginType,
    _config: &serde_json::Value,
) -> bool {
    match execution_site {
        "controller" => true,
        "agent" => false,
        _ => {
            // "auto" — check static capability (no instantiation needed)
            PluginRegistry::capabilities_for(plugin_type.clone())
                .contains(&PluginCapability::ControllerSideFetchReleases)
        }
    }
}

/// Execute controller-side `fetch_releases` for a batch of jobs.
///
/// Groups by `(plugin_type, package_identifier, config)` deduplication has
/// already been applied by the caller — each job represents one distinct API
/// call. Updates `host_software_item.latest_version`,
/// `latest_version_fetched_at`, and `software_item.last_checked_at` for all
/// successful fetches. Pushes MQTT software states after updating.
///
/// Returns the number of jobs for which `fetch_releases` succeeded.
async fn run_controller_fetch_jobs(
    db: &sea_orm::DatabaseConnection,
    notification_service: &crate::notification_service::NotificationService,
    event_broadcaster: &crate::event_broadcaster::EventBroadcaster,
    tenant_id: Uuid,
    jobs: Vec<ControllerFetchJob>,
) -> u32 {
    if jobs.is_empty() {
        return 0;
    }

    let noop_executor: Arc<dyn CommandExecutor> = Arc::new(NoopCommandExecutor);
    let now = OffsetDateTime::now_utc();
    let mut succeeded = 0u32;
    let mut updated_item_ids: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut completed_pairs: Vec<(Uuid, Uuid)> = Vec::new();

    for job in &jobs {
        let plugin = match PluginRegistry::create_plugin(
            job.plugin_type.clone(),
            &job.merged_config,
            noop_executor.clone(),
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    plugin_type = ?job.plugin_type,
                    package = %job.package_identifier,
                    error = %e,
                    "controller-side fetch: failed to create plugin"
                );
                continue;
            }
        };

        let Some(fetcher) = plugin.as_release_fetcher() else {
            tracing::warn!(
                plugin_type = ?job.plugin_type,
                "plugin does not implement ReleaseFetcherPlugin; skipping"
            );
            continue;
        };
        let releases = match fetcher.fetch_releases(&job.package_identifier).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    plugin_type = ?job.plugin_type,
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
                plugin_type = ?job.plugin_type,
                package = %job.package_identifier,
                "controller-side fetch: no releases returned"
            );
            continue;
        };

        let latest_version_str = latest.version.to_string();
        let release_metadata = serde_json::to_value(latest).unwrap_or(serde_json::Value::Null);

        tracing::debug!(
            plugin_type = ?job.plugin_type,
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

// --- Endpoints ---

/// Create a new software item.
#[utoipa::path(
    post,
    path = "/api/v1/software-items",
    request_body = CreateSoftwareItemRequest,
    extensions(("x-required-permission" = json!("create_software"))),
    responses(
        (status = 201, description = "Software item created", body = SoftwareItemResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "Duplicate software item")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_software_item(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanCreateSoftware(_user): CanCreateSoftware,
    Json(req): Json<CreateSoftwareItemRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }
    match item_queries::create_software_item(&tenant_db, req).await {
        Ok(resp) => {
            state
                .event_broadcaster
                .send(
                    tenant_db.tenant_id,
                    AdminEvent::SoftwareItemCreated { id: resp.id },
                )
                .await;
            (StatusCode::CREATED, Json(resp)).into_response()
        }
        Err(e) => query_error_to_response(e),
    }
}

/// List all active software items (with host count).
#[utoipa::path(
    get,
    path = "/api/v1/software-items",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("featured" = Option<bool>, Query, description = "Filter by featured status. Omit to return all items."),
        ("host_id" = Option<Uuid>, Query, description = "Filter by host UUID — only return items assigned to this host."),
        ("updatable" = Option<bool>, Query, description = "Filter by update availability. true = only items with an update available; false = only up-to-date items.")
    ),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Paginated list of software items", body = PaginatedResponse<SoftwareItemResponse>),
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_software_items(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<ListSoftwareItemsParams>,
) -> Response {
    match item_queries::list_software_items(&tenant_db, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list software items: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a software item with assigned hosts and installed versions.
#[utoipa::path(
    get,
    path = "/api/v1/software-items/{id}",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Software item details", body = SoftwareItemDetailResponse),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_software_item(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path(item_id): Path<Uuid>,
) -> Response {
    match item_queries::get_software_item(&tenant_db, item_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Software item not found"),
        Err(e) => {
            tracing::error!("Failed to get software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a software item (partial update).
#[utoipa::path(
    put,
    path = "/api/v1/software-items/{id}",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    request_body = UpdateSoftwareItemRequest,
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 200, description = "Software item updated", body = SoftwareItemResponse),
        (status = 404, description = "Software item not found"),
        (status = 409, description = "Duplicate software item")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_software_item(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Path(item_id): Path<Uuid>,
    Json(req): Json<UpdateSoftwareItemRequest>,
) -> Response {
    match item_queries::update_software_item(&tenant_db, item_id, req).await {
        Ok(resp) => {
            state
                .event_broadcaster
                .send(
                    tenant_db.tenant_id,
                    AdminEvent::SoftwareItemUpdated { id: item_id },
                )
                .await;
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => query_error_to_response(e),
    }
}

/// Soft-delete a software item.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
    ),
    extensions(("x-required-permission" = json!("delete_software"))),
    responses(
        (status = 204, description = "Software item deleted"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_software_item(
    tenant_db: TenantDb,
    CanDeleteSoftware(_user): CanDeleteSoftware,
    Path(item_id): Path<Uuid>,
) -> Response {
    match item_queries::delete_software_item(&tenant_db, item_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Software item not found"),
        Err(e) => {
            tracing::error!("Failed to delete software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Approve a discovered software item by marking it as featured.
///
/// Sets `featured = true` so the item appears in MQTT software state entities
/// and update management dashboards.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/approve",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 200, description = "Software item approved", body = SoftwareItemResponse),
        (status = 404, description = "Software item not found"),
        (status = 409, description = "Item is already featured")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn approve_software_item(
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Path(item_id): Path<Uuid>,
) -> Response {
    let item = match software_item::Entity::find_by_id(item_id)
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(i)) => i,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
        Err(e) => {
            tracing::error!("Failed to fetch software item: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if item.featured {
        return error_response(StatusCode::CONFLICT, "Software item is already featured");
    }

    let now = OffsetDateTime::now_utc();
    let mut active: software_item::ActiveModel = item.into();
    active.featured = Set(true);
    active.updated_at = Set(now);

    let updated = match active.update(tenant_db.db()).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to approve software item: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match item_queries::get_software_item(&tenant_db, updated.id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Item not found after update",
        ),
        Err(e) => {
            tracing::error!("Failed to fetch approved software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Assign a software item to additional hosts.
///
/// Each host in `host_assignments` carries its own `plugin_config_id`,
/// `package_identifier`, and optional `config_override`.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    request_body = AssignHostsRequest,
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 200, description = "Hosts assigned", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn assign_hosts(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Path(item_id): Path<Uuid>,
    Json(req): Json<AssignHostsRequest>,
) -> Response {
    if req.host_assignments.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "host_assignments must not be empty",
        );
    }

    match item_queries::assign_hosts(state.plugin_ops.as_ref(), &tenant_db, item_id, req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => query_error_to_response(e),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct DeleteHostAssignmentParams {
    pub ignore: Option<bool>,
}

/// Unassign a software item from a host.
///
/// The optional `ignore=true` query parameter also creates a tenant-wide
/// autodiscovery ignore rule by the software item's display name, preventing
/// all future re-discovery of items with that name regardless of which plugin
/// config or target produced them.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}/hosts/{host_id}",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("ignore" = Option<bool>, Query, description = "If true, permanently suppress items with this name from future autodiscovery runs")
    ),
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 204, description = "Host unassigned"),
        (status = 404, description = "Software item or host assignment not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn unassign_host(
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<DeleteHostAssignmentParams>,
) -> Response {
    // If ignore=true, load the software item name before deleting so we can
    // create a name-based ignore rule.
    let ignore_name: Option<String> = if params.ignore.unwrap_or(false) {
        match SoftwareItem::find_by_id(item_id)
            .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
            .one(tenant_db.db())
            .await
        {
            Ok(Some(item)) => Some(item.name),
            Ok(None) => {
                return error_response(StatusCode::NOT_FOUND, "Software item not found");
            }
            Err(e) => {
                tracing::error!("Failed to look up software item for ignore: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    } else {
        None
    };

    match item_queries::unassign_host(&tenant_db, item_id, host_id).await {
        Ok(true) => {
            if let Some(name) = ignore_name
                && let Err(e) = autodiscovery_queries::create_or_ignore_ignore_rule(
                    tenant_db.db(),
                    tenant_db.tenant_id,
                    &name,
                    None,
                )
                .await
            {
                tracing::warn!("Failed to create autodiscovery ignore rule: {e}");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error_response(
            StatusCode::NOT_FOUND,
            "Software item or host assignment not found",
        ),
        Err(e) => {
            tracing::error!("Failed to unassign host from software item: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update the plugin assignment for a specific host–software-item link.
#[utoipa::path(
    put,
    path = "/api/v1/software-items/{id}/hosts/{host_id}",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = UpdateHostAssignmentRequest,
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 200, description = "Host assignment updated", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item or host assignment not found"),
        (status = 409, description = "Duplicate host assignment")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_host_assignment(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateHostAssignmentRequest>,
) -> Response {
    match item_queries::update_host_assignment(
        state.plugin_ops.as_ref(),
        &tenant_db,
        item_id,
        host_id,
        req,
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => query_error_to_response(e),
    }
}

/// Trigger a software update for a specific host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/update",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    request_body = TriggerUpdateRequest,
    extensions(("x-required-permission" = json!("trigger_updates"))),
    responses(
        (status = 200, description = "Update triggered", body = TriggerUpdateResponse),
        (status = 400, description = "Invalid input or validation failed"),
        (status = 404, description = "Software item, host, or agent not found"),
        (status = 409, description = "Update already in progress")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn trigger_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerUpdates(user): CanTriggerUpdates,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<TriggerUpdateRequest>,
) -> Response {
    // Convert the API release_info type to the wire type before delegating.
    let release_info = req
        .release_info
        .map(|ri| uptrakit_internal_wire::ReleaseInfo {
            tag: ri.tag,
            release_url: ri.release_url,
            assets: ri
                .assets
                .into_iter()
                .map(|a| uptrakit_internal_wire::ReleaseAsset {
                    name: a.name,
                    download_url: a.download_url,
                    size: a.size,
                    content_type: None,
                    sha256_digest: None,
                })
                .collect(),
            // Attestation fields are enriched server-side at dispatch time from
            // latest_release_metadata and the fetch_releases plugin config.
            attestation_status: None,
            require_attestation: false,
        });

    match crate::queries::update_triggers::trigger_update_for_host(
        tenant_db.db(),
        &state.notification_service,
        crate::queries::update_triggers::TriggerUpdateParams {
            tenant_id: tenant_db.tenant_id,
            item_id,
            host_id,
            to_version: req.to_version,
            actor_type: ActorType::User,
            actor_id: &user.user_id.to_string(),
            release_info,
            interactive: req.interactive,
        },
    )
    .await
    {
        Ok(result) => {
            let status = match result.initial_status {
                uptrakit_shared_db::entity::update_history::UpdateStatus::Pending => {
                    TriggerUpdateStatus::Pending
                }
                _ => TriggerUpdateStatus::Queued,
            };
            // Push updated software states immediately so that any connected
            // MQTT/HA entity transitions to `in_progress: true`.
            state
                .notification_service
                .push_software_states_for_tenant(tenant_db.db(), tenant_db.tenant_id)
                .await;
            // Notify SSE subscribers so the History page shows the new entry
            // immediately, without waiting for the agent's UpdateStarted message.
            state
                .event_broadcaster
                .send(
                    tenant_db.tenant_id,
                    AdminEvent::UpdateTriggered {
                        update_history_id: result.update_history_id,
                        host_id,
                        software_item_id: item_id,
                    },
                )
                .await;
            let resp = TriggerUpdateResponse {
                update_history_id: result.update_history_id,
                status,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(report) => trigger_update_error_response(&report),
    }
}

/// Map a [`TriggerUpdateError`] to an HTTP error response with the appropriate
/// status code and user-facing message.
fn trigger_update_error_response(
    report: &rootcause::Report<crate::queries::update_dispatch::TriggerUpdateError>,
) -> Response {
    use crate::queries::update_dispatch::TriggerUpdateError;

    let (status, msg) = match report.current_context() {
        TriggerUpdateError::SoftwareItemNotFound => {
            (StatusCode::NOT_FOUND, "Software item not found")
        }
        TriggerUpdateError::HostNotFound => (StatusCode::NOT_FOUND, "Host not found"),
        TriggerUpdateError::HostNotAssigned => (
            StatusCode::BAD_REQUEST,
            "Host is not assigned to this software item",
        ),
        TriggerUpdateError::NoAgent => (StatusCode::NOT_FOUND, "No agent linked to this host"),
        TriggerUpdateError::AgentNotApproved => (StatusCode::BAD_REQUEST, "Agent is not approved"),
        TriggerUpdateError::UpdateAlreadyActive => (
            StatusCode::CONFLICT,
            "An update is already pending or in progress",
        ),
        TriggerUpdateError::NoExecuteUpdatePlugin => (
            StatusCode::BAD_REQUEST,
            "No execute_update plugin assigned for this host",
        ),
        TriggerUpdateError::PluginConfigNotFound => {
            (StatusCode::INTERNAL_SERVER_ERROR, "Plugin config not found")
        }
        TriggerUpdateError::UnknownPluginType(pt) => {
            tracing::error!("Unknown plugin type: {pt}");
            (StatusCode::BAD_REQUEST, "Unknown plugin type")
        }
        TriggerUpdateError::Database(_) => {
            tracing::error!("Database error in trigger_update: {report}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    };
    error_response(status, msg)
}

/// Trigger a version check for a specific software item across all assigned hosts.
///
/// Each host receives a version-check message using its own per-host plugin config
/// and package identifier.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/check-versions",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("trigger_checks"))),
    responses(
        (status = 200, description = "Version check triggered", body = TriggerVersionCheckResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found or no agents")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn check_versions(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerChecks(_user): CanTriggerChecks,
    Path(item_id): Path<Uuid>,
) -> Response {
    // Verify software item exists and is active
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
            Some(i) => i,
            None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
        };

    // Phase 1: Load all data needed for version checks.
    let ctx = match load_version_check_context(&tenant_db, item_id).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    // Phase 2: Collect and run controller-side fetch_releases jobs.
    let controller_checks_run = collect_and_run_controller_fetches(&tenant_db, &state, &ctx).await;

    // Phase 3: Send CheckVersions messages to agents for agent-side assignments.
    let agents_notified = dispatch_agent_version_checks(&state, &ctx, item_id, &item.name).await;

    if agents_notified == 0 && controller_checks_run == 0 {
        return error_response(
            StatusCode::NOT_FOUND,
            "No approved agents found for assigned hosts",
        );
    }

    let message = match (agents_notified, controller_checks_run) {
        (a, 0) => format!(
            "Version check triggered for '{}' on {a} agent(s)",
            item.name
        ),
        (0, c) => format!(
            "Version check completed for '{}' ({c} controller-side fetch(es))",
            item.name
        ),
        (a, c) => format!(
            "Version check triggered for '{}' on {a} agent(s) and {c} controller-side fetch(es)",
            item.name
        ),
    };

    let resp = TriggerVersionCheckResponse {
        agents_notified,
        controller_checks_run,
        message,
    };
    (StatusCode::OK, Json(resp)).into_response()
}

/// All pre-loaded data needed to dispatch version checks across hosts.
struct VersionCheckContext {
    links: Vec<host_software_item::Model>,
    plugin_assignments: Vec<host_software_item_plugin::Model>,
    hosts: HashMap<Uuid, host::Model>,
    service_hosts: HashMap<Uuid, Uuid>,
    configs: HashMap<Uuid, plugin_config::Model>,
    plugin_by_host_role: HashMap<(Uuid, String), usize>,
}

impl VersionCheckContext {
    /// Build a `PluginAssignment` from a plugin row and its (optional) config.
    fn build_assignment(
        &self,
        plugin: &host_software_item_plugin::Model,
    ) -> Option<uptrakit_internal_wire::PluginAssignment> {
        let config_model = plugin
            .plugin_config_id
            .and_then(|pc_id| self.configs.get(&pc_id));
        let plugin_type_str = config_model
            .map(|c| c.plugin_type.clone())
            .unwrap_or_else(|| plugin.plugin_type.clone());
        let plugin_type: uptrakit_internal_wire::PluginType =
            serde_json::from_value(serde_json::Value::String(plugin_type_str)).ok()?;
        let merged = uptrakit_config_merge::resolve_effective_config(
            None,
            config_model.map(|c| &c.config),
            plugin.config.as_ref(),
        );
        Some(uptrakit_internal_wire::PluginAssignment {
            plugin_type,
            package_identifier: plugin.package_identifier.clone(),
            config: merged,
        })
    }

    /// Look up a plugin assignment by (host_id, role).
    fn get_plugin(&self, host_id: Uuid, role: &str) -> Option<&host_software_item_plugin::Model> {
        self.plugin_by_host_role
            .get(&(host_id, role.to_string()))
            .map(|&idx| &self.plugin_assignments[idx])
    }
}

/// Load all data needed to dispatch version checks: host links, plugin
/// assignments, hosts, service mappings, and plugin configs.
async fn load_version_check_context(
    tenant_db: &TenantDb,
    item_id: Uuid,
) -> std::result::Result<VersionCheckContext, Response> {
    let links = HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .all(tenant_db.db())
        .await
        .map_err(|e| {
            tracing::error!("Failed to load software item hosts: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?;

    if links.is_empty() {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "No hosts assigned to this software item",
        ));
    }

    let plugin_assignments = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.is_in(["detect_version", "fetch_releases"]))
        .all(tenant_db.db())
        .await
        .map_err(|e| {
            tracing::error!("Failed to load plugin assignments: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?;

    let host_ids: Vec<Uuid> = links.iter().map(|l| l.host_id).collect();
    let config_ids: Vec<Uuid> = plugin_assignments
        .iter()
        .filter_map(|p| p.plugin_config_id)
        .collect();

    let hosts: HashMap<Uuid, host::Model> = tenant_db
        .find::<host::Entity>()
        .filter(host::Column::Id.is_in(host_ids.clone()))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .map_err(|e| {
            tracing::error!("Failed to load hosts: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?
        .into_iter()
        .map(|h| (h.id, h))
        .collect();

    let service_hosts: HashMap<Uuid, Uuid> = tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.is_in(host_ids))
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(service::Column::Status.eq(service::ServiceStatus::Approved))
        .all(tenant_db.db())
        .await
        .map_err(|e| {
            tracing::error!("Failed to load service-host links: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        })?
        .into_iter()
        .map(|sh| (sh.host_id, sh.service_id))
        .collect();

    let configs: HashMap<Uuid, plugin_config::Model> = if config_ids.is_empty() {
        HashMap::new()
    } else {
        tenant_db
            .find::<plugin_config::Entity>()
            .filter(plugin_config::Column::Id.is_in(config_ids))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .all(tenant_db.db())
            .await
            .map_err(|e| {
                tracing::error!("Failed to load plugin configs: {e}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            })?
            .into_iter()
            .map(|c| (c.id, c))
            .collect()
    };

    // Index plugin assignments by (host_id, role) → index into plugin_assignments vec.
    let mut plugin_by_host_role: HashMap<(Uuid, String), usize> = HashMap::new();
    for (idx, pa) in plugin_assignments.iter().enumerate() {
        plugin_by_host_role.insert((pa.host_id, pa.role.clone()), idx);
    }

    Ok(VersionCheckContext {
        links,
        plugin_assignments,
        hosts,
        service_hosts,
        configs,
        plugin_by_host_role,
    })
}

/// Collect controller-side fetch_releases jobs (deduplicated) and execute them.
async fn collect_and_run_controller_fetches(
    tenant_db: &TenantDb,
    state: &Arc<AppState>,
    ctx: &VersionCheckContext,
) -> u32 {
    let mut controller_job_map: HashMap<(String, String), ControllerFetchJob> = HashMap::new();
    for pa in ctx
        .plugin_assignments
        .iter()
        .filter(|pa| pa.role == "fetch_releases")
    {
        let config_model = pa
            .plugin_config_id
            .and_then(|pc_id| ctx.configs.get(&pc_id));
        let plugin_type_str = config_model
            .map(|c| c.plugin_type.clone())
            .unwrap_or_else(|| pa.plugin_type.clone());
        let Ok(plugin_type) =
            serde_json::from_value::<PluginType>(serde_json::Value::String(plugin_type_str))
        else {
            continue;
        };
        let merged = uptrakit_config_merge::resolve_effective_config(
            None,
            config_model.map(|c| &c.config),
            pa.config.as_ref(),
        );
        if is_controller_fetch_site(&pa.execution_site, &plugin_type, &merged) {
            let dedup_key = pa
                .plugin_config_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| pa.plugin_type.clone());
            let key = (dedup_key, pa.package_identifier.clone());
            controller_job_map
                .entry(key)
                .or_insert_with(|| ControllerFetchJob {
                    plugin_type: plugin_type.clone(),
                    package_identifier: pa.package_identifier.clone(),
                    merged_config: merged,
                    targets: Vec::new(),
                })
                .targets
                .push((pa.host_id, pa.software_item_id));
        }
    }

    run_controller_fetch_jobs(
        tenant_db.db(),
        &state.notification_service,
        &state.event_broadcaster,
        tenant_db.tenant_id,
        controller_job_map.into_values().collect(),
    )
    .await
}

/// Send CheckVersions messages to agents for agent-side assignments.
/// Returns the number of agents notified.
async fn dispatch_agent_version_checks(
    state: &Arc<AppState>,
    ctx: &VersionCheckContext,
    item_id: Uuid,
    item_name: &str,
) -> u32 {
    let mut agents_notified: u32 = 0;
    let mut seen = std::collections::HashSet::new();

    for link in &ctx.links {
        let Some(host_record) = ctx.hosts.get(&link.host_id) else {
            continue;
        };
        let Some(&service_id) = ctx.service_hosts.get(&link.host_id) else {
            continue;
        };
        if !seen.insert((service_id, link.host_id)) {
            continue;
        }

        let detect_version = ctx
            .get_plugin(link.host_id, "detect_version")
            .and_then(|p| ctx.build_assignment(p));

        let fetch_releases = ctx
            .get_plugin(link.host_id, "fetch_releases")
            .and_then(|p| {
                let config_model = p.plugin_config_id.and_then(|pc_id| ctx.configs.get(&pc_id));
                let plugin_type_str = config_model
                    .map(|c| c.plugin_type.clone())
                    .unwrap_or_else(|| p.plugin_type.clone());
                let plugin_type: PluginType =
                    serde_json::from_value(serde_json::Value::String(plugin_type_str)).ok()?;
                let merged = uptrakit_config_merge::resolve_effective_config(
                    None,
                    config_model.map(|c| &c.config),
                    p.config.as_ref(),
                );
                // Skip assignments that ran (or will run) controller-side.
                if is_controller_fetch_site(&p.execution_site, &plugin_type, &merged) {
                    None
                } else {
                    ctx.build_assignment(p)
                }
            });

        // No agent-side work for this host — controller-side fetch handled it.
        if detect_version.is_none() && fetch_releases.is_none() {
            continue;
        }

        let assignment = uptrakit_internal_wire::VersionCheckAssignment {
            software_item_id: item_id,
            name: item_name.to_string(),
            detect_version,
            fetch_releases,
            host_software_item_id: Some(link.id),
        };

        let msg = uptrakit_internal_wire::ControllerMessage::CheckVersions(
            uptrakit_internal_wire::CheckVersionsPayload {
                host_machine_id: host_record.machine_id.clone(),
                assignments: vec![assignment],
            },
        );
        state.notification_service.send(&service_id, msg).await;
        agents_notified += 1;
    }

    agents_notified
}

/// Trigger a version check for a specific software item on a specific host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/check-versions",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    extensions(("x-required-permission" = json!("trigger_checks"))),
    responses(
        (status = 200, description = "Version check triggered", body = TriggerVersionCheckResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item, host, or agent not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn check_versions_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerChecks(_user): CanTriggerChecks,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
) -> Response {
    // Verify software item exists and is active
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
            Some(i) => i,
            None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
        };

    // Verify host exists and belongs to tenant; keep the record for machine_id.
    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("Failed to lookup host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify host is assigned
    let link = match item_queries::load_host_assignment(tenant_db.db(), host_id, item_id).await {
        Some(l) => l,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item",
            );
        }
    };

    // Find agent linked to host
    let agent_link = match ServiceHost::find()
        .filter(service_host::Column::HostId.eq(host_id))
        .one(tenant_db.db())
        .await
    {
        Ok(Some(l)) => l,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "No agent linked to this host");
        }
        Err(e) => {
            tracing::error!("Failed to find agent for host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify agent exists and is approved
    let agent = match Service::find_by_id(agent_link.service_id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(a)) => {
            if a.status != service::ServiceStatus::Approved {
                return error_response(StatusCode::BAD_REQUEST, "Agent is not approved");
            }
            a
        }
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "Agent not found or deactivated");
        }
        Err(e) => {
            tracing::error!("Failed to lookup agent: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Load role-specific plugin assignments for this host
    let role_plugins = match HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.is_in(["detect_version", "fetch_releases"]))
        .all(tenant_db.db())
        .await
    {
        Ok(ps) => ps,
        Err(e) => {
            tracing::error!("Failed to load role plugins: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Build role assignments, separating controller-side from agent-side.
    let mut detect_version: Option<uptrakit_internal_wire::PluginAssignment> = None;
    let mut fetch_releases: Option<uptrakit_internal_wire::PluginAssignment> = None;
    let mut controller_fetch_jobs: Vec<ControllerFetchJob> = Vec::new();

    for plugin in &role_plugins {
        let config = match plugin.plugin_config_id {
            Some(pc_id) => match find_raw_active_config(&tenant_db, pc_id).await {
                Ok(Some(c)) => Some(c),
                Ok(None) => {
                    tracing::warn!(
                        plugin_config_id = %pc_id,
                        "plugin config not found or deactivated, skipping role assignment"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        plugin_config_id = %pc_id,
                        error = %e,
                        "DB error loading plugin config, skipping role assignment"
                    );
                    continue;
                }
            },
            None => None,
        };
        let plugin_type_str = config
            .as_ref()
            .map(|c| c.plugin_type.clone())
            .unwrap_or_else(|| plugin.plugin_type.clone());
        let Ok(plugin_type) =
            serde_json::from_value::<PluginType>(serde_json::Value::String(plugin_type_str))
        else {
            tracing::error!("Unknown plugin type: {}", plugin.plugin_type);
            continue;
        };
        let merged = uptrakit_config_merge::resolve_effective_config(
            None,
            config.as_ref().map(|c| &c.config),
            plugin.config.as_ref(),
        );
        let pa = uptrakit_internal_wire::PluginAssignment {
            plugin_type: plugin_type.clone(),
            package_identifier: plugin.package_identifier.clone(),
            config: merged.clone(),
        };
        match plugin.role.as_str() {
            "detect_version" => detect_version = Some(pa),
            "fetch_releases" => {
                if is_controller_fetch_site(&plugin.execution_site, &plugin_type, &merged) {
                    controller_fetch_jobs.push(ControllerFetchJob {
                        plugin_type,
                        package_identifier: plugin.package_identifier.clone(),
                        merged_config: merged,
                        targets: vec![(host_id, item_id)],
                    });
                } else {
                    fetch_releases = Some(pa);
                }
            }
            _ => {}
        }
    }

    // Run controller-side fetch_releases (e.g. GitHub, Docker).
    let controller_checks_run = run_controller_fetch_jobs(
        tenant_db.db(),
        &state.notification_service,
        &state.event_broadcaster,
        tenant_db.tenant_id,
        controller_fetch_jobs,
    )
    .await;

    // If no agent-side work is needed, return immediately.
    if detect_version.is_none() && fetch_releases.is_none() {
        if controller_checks_run > 0 {
            let resp = TriggerVersionCheckResponse {
                agents_notified: 0,
                controller_checks_run,
                message: format!(
                    "Version check completed for '{}' (controller-side)",
                    item.name
                ),
            };
            return (StatusCode::OK, Json(resp)).into_response();
        }
        return error_response(
            StatusCode::BAD_REQUEST,
            "No detect_version or fetch_releases plugin assigned",
        );
    }

    let assignment = uptrakit_internal_wire::VersionCheckAssignment {
        software_item_id: item_id,
        name: item.name.clone(),
        detect_version,
        fetch_releases,
        host_software_item_id: Some(link.id),
    };

    let msg = uptrakit_internal_wire::ControllerMessage::CheckVersions(
        uptrakit_internal_wire::CheckVersionsPayload {
            host_machine_id: host_record.machine_id.clone(),
            assignments: vec![assignment],
        },
    );
    state.notification_service.send(&agent.id, msg).await;

    let resp = TriggerVersionCheckResponse {
        agents_notified: 1,
        controller_checks_run,
        message: format!("Version check triggered for '{}' on 1 agent", item.name),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

/// Perform a batch action on multiple software items.
///
/// Supported actions: `approve`, `delete`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/software-items/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Software Items",
    extensions(("x-required-permission" = json!("delete_software"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_software_items(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanDeleteSoftware(_user): CanDeleteSoftware,
    Json(body): Json<BatchActionRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => {
            // Inline batch approve: set featured = true for matching items.
            let now = OffsetDateTime::now_utc();
            let mut succeeded = Vec::new();
            let mut failures: Vec<(Uuid, String)> = Vec::new();
            for &id in &body.ids {
                match software_item::Entity::find_by_id(id)
                    .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
                    .filter(software_item::Column::DeactivatedAt.is_null())
                    .one(tenant_db.db())
                    .await
                {
                    Ok(Some(item)) => {
                        if item.featured {
                            // Already featured -- still count as success (idempotent).
                            succeeded.push(id);
                            continue;
                        }
                        let mut active: software_item::ActiveModel = item.into();
                        active.featured = Set(true);
                        active.updated_at = Set(now);
                        match active.update(tenant_db.db()).await {
                            Ok(_) => succeeded.push(id),
                            Err(e) => failures.push((id, e.to_string())),
                        }
                    }
                    Ok(None) => failures.push((id, "not found".to_string())),
                    Err(e) => failures.push((id, e.to_string())),
                }
            }
            (succeeded, failures)
        }
        "delete" => match item_queries::batch_delete_software_items(&tenant_db, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch delete failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        unknown => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: approve, delete"),
            );
        }
    };

    // Dispatch side effects per succeeded item.
    for id in &succeeded_ids {
        state
            .event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::SoftwareItemUpdated { id: *id },
            )
            .await;
    }

    let response = BatchActionResponse {
        succeeded: succeeded_ids
            .into_iter()
            .map(|id| BatchActionSuccess { id })
            .collect(),
        failed: failed
            .into_iter()
            .map(|(id, error)| BatchActionFailure { id, error })
            .collect(),
    };

    (StatusCode::OK, Json(response)).into_response()
}
