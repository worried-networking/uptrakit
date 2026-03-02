use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSoftware, CanViewSoftware};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::plugin_configs::find_raw_active_config;
use crate::queries::software_items::{self as item_queries, SoftwareItemQueryError};
use crate::tenant_db::TenantDb;
use async_trait::async_trait;
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
use tokio::sync::mpsc;
use uptrakit_command::{CommandOutput, CommandSpec, UpdateOutputLine};
use uptrakit_plugin_infrastructure_registry::{
    CommandExecutor, PluginCapability, PluginRegistry, PluginType,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, service,
    service_host, software_item,
};
use uptrakit_shared_types::SoftwareDiscoveryState;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

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

/// Controller-side command executor stub.
///
/// The controller never executes local shell commands. API-based plugins
/// (GitHub, Docker) perform HTTP calls internally and never invoke the executor.
/// This struct satisfies the `Arc<dyn CommandExecutor>` requirement of
/// `PluginRegistry::create_plugin` without pulling in a real executor.
struct NoopCommandExecutor;

#[async_trait]
impl CommandExecutor for NoopCommandExecutor {
    async fn execute(
        &self,
        _: &CommandSpec,
        _: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_command::Result<CommandOutput> {
        unreachable!("NoopCommandExecutor::execute called on the controller — this is a bug")
    }

    async fn execute_quiet(&self, _: &CommandSpec) -> uptrakit_command::Result<CommandOutput> {
        unreachable!("NoopCommandExecutor::execute_quiet called on the controller — this is a bug")
    }
}

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

        let releases = match plugin.fetch_releases(&job.package_identifier).await {
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

        let category_str = latest
            .category
            .clone()
            .unwrap_or_default()
            .to_string();

        for (host_id, software_item_id) in &job.targets {
            let active = host_software_item::ActiveModel {
                host_id: Set(*host_id),
                software_item_id: Set(*software_item_id),
                latest_version: Set(Some(latest_version_str.clone())),
                latest_version_fetched_at: Set(Some(now)),
                latest_release_metadata: Set(Some(release_metadata.clone())),
                update_category: Set(category_str.clone()),
                ..Default::default()
            };
            if let Err(e) = active.update(db).await {
                tracing::warn!(
                    host_id = %host_id,
                    software_item_id = %software_item_id,
                    error = %e,
                    "controller-side fetch: failed to update host_software_item"
                );
            } else {
                updated_item_ids.insert(*software_item_id);
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
    }

    succeeded
}

// --- Endpoints ---

/// Create a new software item.
#[utoipa::path(
    post,
    path = "/api/v1/software-items",
    request_body = CreateSoftwareItemRequest,
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 201, description = "Software item created", body = SoftwareItemResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "Duplicate software item")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn create_software_item(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Json(req): Json<CreateSoftwareItemRequest>,
) -> Response {
    if let Err(e) = req.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }
    match item_queries::create_software_item(&tenant_db, req).await {
        Ok(resp) => (StatusCode::CREATED, Json(resp)).into_response(),
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
        ("discovery_state" = Option<String>, Query, description = "Filter by discovery state: \"pending\" or \"approved\". Omit to return all items.")
    ),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Paginated list of software items", body = PaginatedResponse<SoftwareItemResponse>),
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Software item updated", body = SoftwareItemResponse),
        (status = 404, description = "Software item not found"),
        (status = 409, description = "Duplicate software item")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn update_software_item(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(item_id): Path<Uuid>,
    Json(req): Json<UpdateSoftwareItemRequest>,
) -> Response {
    match item_queries::update_software_item(&tenant_db, item_id, req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 204, description = "Software item deleted"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn delete_software_item(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
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

/// Approve a pending discovered software item.
///
/// Sets `discovery_state = 'approved'` and enables the item for version
/// tracking and update management.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/approve",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Software item approved", body = SoftwareItemResponse),
        (status = 404, description = "Software item not found"),
        (status = 409, description = "Item is not in pending discovery state")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn approve_software_item(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
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

    if item.discovery_state != Some(SoftwareDiscoveryState::Pending) {
        return error_response(
            StatusCode::CONFLICT,
            "Software item is not in pending discovery state",
        );
    }

    let now = OffsetDateTime::now_utc();
    let mut active: software_item::ActiveModel = item.into();
    active.discovery_state = Set(Some(SoftwareDiscoveryState::Approved));
    active.enabled = Set(true);
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Hosts assigned", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn assign_hosts(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(item_id): Path<Uuid>,
    Json(req): Json<AssignHostsRequest>,
) -> Response {
    if req.host_assignments.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "host_assignments must not be empty",
        );
    }

    match item_queries::assign_hosts(&tenant_db, item_id, req).await {
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
/// The optional `ignore=true` query parameter also creates an autodiscovery
/// ignore rule based on the host assignment's plugin config and package
/// identifier, so this combination is not re-discovered in future runs.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}/hosts/{host_id}",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("ignore" = Option<bool>, Query, description = "If true, permanently suppress this package/plugin combination from future autodiscovery runs")
    ),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 204, description = "Host unassigned"),
        (status = 404, description = "Software item or host assignment not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn unassign_host(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<DeleteHostAssignmentParams>,
) -> Response {
    // If ignore=true, load the detect_version role plugin assignment before
    // deleting so we can capture plugin_config_id + package_identifier for the
    // autodiscovery ignore rule.
    let ignore_info: Option<(uuid::Uuid, String)> = if params.ignore.unwrap_or(false) {
        // Load the detect_version role plugin assignment for ignore rule creation.
        match HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
            .filter(host_software_item_plugin::Column::Role.eq("detect_version"))
            .one(tenant_db.db())
            .await
        {
            Ok(Some(plugin)) => Some((plugin.plugin_config_id, plugin.package_identifier)),
            Ok(None) => {
                // No detect_version plugin -- try any role to get plugin info
                match HostSoftwareItemPlugin::find()
                    .filter(host_software_item_plugin::Column::HostId.eq(host_id))
                    .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
                    .one(tenant_db.db())
                    .await
                {
                    Ok(Some(plugin)) => Some((plugin.plugin_config_id, plugin.package_identifier)),
                    Ok(None) => {
                        return error_response(StatusCode::NOT_FOUND, "Host assignment not found");
                    }
                    Err(e) => {
                        tracing::error!("Failed to look up host assignment for ignore: {e}");
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to look up host assignment for ignore: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    } else {
        None
    };

    match item_queries::unassign_host(&tenant_db, item_id, host_id).await {
        Ok(true) => {
            if let Some((plugin_config_id, package_identifier)) = ignore_info
                && let Err(e) = autodiscovery_queries::create_or_ignore_ignore_rule(
                    tenant_db.db(),
                    tenant_db.tenant_id,
                    plugin_config_id,
                    &package_identifier,
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Host assignment updated", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item or host assignment not found"),
        (status = 409, description = "Duplicate host assignment")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn update_host_assignment(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateHostAssignmentRequest>,
) -> Response {
    match item_queries::update_host_assignment(&tenant_db, item_id, host_id, req).await {
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Update triggered", body = TriggerUpdateResponse),
        (status = 400, description = "Invalid input or validation failed"),
        (status = 404, description = "Software item, host, or agent not found"),
        (status = 409, description = "Update already in progress")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn trigger_update(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(user): CanManageSoftware,
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
                })
                .collect(),
        });

    match crate::queries::update_triggers::trigger_update_for_host(
        tenant_db.db(),
        &state.notification_service,
        crate::queries::update_triggers::TriggerUpdateParams {
            tenant_id: tenant_db.tenant_id,
            item_id,
            host_id,
            to_version: req.to_version,
            actor_type: "user",
            actor_id: &user.user_id.to_string(),
            release_info,
        },
    )
    .await
    {
        Ok(result) => {
            let status = if result.agent_connected {
                TriggerUpdateStatus::Pending
            } else {
                TriggerUpdateStatus::Queued
            };
            // Push updated software states immediately so that any connected
            // MQTT/HA entity transitions to `in_progress: true`.
            state
                .notification_service
                .push_software_states_for_tenant(tenant_db.db(), tenant_db.tenant_id)
                .await;
            let resp = TriggerUpdateResponse {
                update_history_id: result.update_history_id,
                status,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(report) => {
            use crate::queries::update_triggers::TriggerUpdateError;
            match report.current_context() {
                TriggerUpdateError::SoftwareItemNotFound => {
                    error_response(StatusCode::NOT_FOUND, "Software item not found")
                }
                TriggerUpdateError::HostNotFound => {
                    error_response(StatusCode::NOT_FOUND, "Host not found")
                }
                TriggerUpdateError::HostNotAssigned => {
                    error_response(
                        StatusCode::BAD_REQUEST,
                        "Host is not assigned to this software item",
                    )
                }
                TriggerUpdateError::NoAgent => {
                    error_response(StatusCode::NOT_FOUND, "No agent linked to this host")
                }
                TriggerUpdateError::AgentNotApproved => {
                    error_response(StatusCode::BAD_REQUEST, "Agent is not approved")
                }
                TriggerUpdateError::UpdateAlreadyActive => {
                    error_response(
                        StatusCode::CONFLICT,
                        "An update is already pending or in progress",
                    )
                }
                TriggerUpdateError::NoExecuteUpdatePlugin => {
                    error_response(
                        StatusCode::BAD_REQUEST,
                        "No execute_update plugin assigned for this host",
                    )
                }
                TriggerUpdateError::PluginConfigNotFound => {
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Plugin config not found")
                }
                TriggerUpdateError::UnknownPluginType(pt) => {
                    tracing::error!("Unknown plugin type: {pt}");
                    error_response(StatusCode::BAD_REQUEST, "Unknown plugin type")
                }
                TriggerUpdateError::Database(_) => {
                    tracing::error!("Database error in trigger_update: {report}");
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
            }
        }
    }
}

/// Trigger a version check for a specific software item across all assigned hosts.
///
/// Each host receives a version-check message using its own per-host plugin config
/// and package identifier.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/check-versions",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Version check triggered", body = TriggerVersionCheckResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found or no agents")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn check_versions(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(item_id): Path<Uuid>,
) -> Response {
    // Verify software item exists and is active
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
            Some(i) => i,
            None => return error_response(StatusCode::NOT_FOUND, "Software item not found"),
        };

    // Find all hosts assigned to this software item
    let links = match HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .all(tenant_db.db())
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::error!("Failed to load software item hosts: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if links.is_empty() {
        return error_response(
            StatusCode::NOT_FOUND,
            "No hosts assigned to this software item",
        );
    }

    // Load all plugin role assignments for all hosts of this software item.
    let plugin_assignments = match HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.is_in(["detect_version", "fetch_releases"]))
        .all(tenant_db.db())
        .await
    {
        Ok(pas) => pas,
        Err(e) => {
            tracing::error!("Failed to load plugin assignments: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Collect distinct config IDs and host IDs.
    let host_ids: Vec<uuid::Uuid> = links.iter().map(|l| l.host_id).collect();
    let config_ids: Vec<uuid::Uuid> = plugin_assignments
        .iter()
        .map(|p| p.plugin_config_id)
        .collect();

    // Batch query: Hosts (tenant-scoped).
    let hosts: std::collections::HashMap<uuid::Uuid, host::Model> = match tenant_db
        .find::<host::Entity>()
        .filter(host::Column::Id.is_in(host_ids.clone()))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(hs) => hs.into_iter().map(|h| (h.id, h)).collect(),
        Err(e) => {
            tracing::error!("Failed to load hosts: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Batch query: service_host -> service JOIN.
    let service_hosts: std::collections::HashMap<uuid::Uuid, uuid::Uuid> = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.is_in(host_ids))
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(service::Column::Status.eq(service::ServiceStatus::Approved))
        .all(tenant_db.db())
        .await
    {
        Ok(shs) => shs
            .into_iter()
            .map(|sh| (sh.host_id, sh.service_id))
            .collect(),
        Err(e) => {
            tracing::error!("Failed to load service-host links: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Batch query: Plugin configs (tenant-scoped).
    let configs: std::collections::HashMap<uuid::Uuid, plugin_config::Model> = if config_ids
        .is_empty()
    {
        std::collections::HashMap::new()
    } else {
        match tenant_db
            .find::<plugin_config::Entity>()
            .filter(plugin_config::Column::Id.is_in(config_ids))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .all(tenant_db.db())
            .await
        {
            Ok(cs) => cs.into_iter().map(|c| (c.id, c)).collect(),
            Err(e) => {
                tracing::error!("Failed to load plugin configs: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    };

    // Index plugin assignments by (host_id, role).
    let mut plugin_by_host_role: HashMap<(Uuid, String), &host_software_item_plugin::Model> =
        HashMap::new();
    for pa in &plugin_assignments {
        plugin_by_host_role.insert((pa.host_id, pa.role.clone()), pa);
    }

    // Helper to build a PluginAssignment from a plugin row and its config.
    let build_assignment =
        |plugin: &host_software_item_plugin::Model| -> Option<uptrakit_internal_wire::PluginAssignment> {
            let config_model = configs.get(&plugin.plugin_config_id)?;
            let plugin_type: uptrakit_internal_wire::PluginType = serde_json::from_value(
                serde_json::Value::String(config_model.plugin_type.clone()),
            )
            .ok()?;
            let merged = uptrakit_update_hooks::merge_config(
                &config_model.config,
                plugin.config_override.as_ref(),
            );
            Some(uptrakit_internal_wire::PluginAssignment {
                plugin_type,
                package_identifier: plugin.package_identifier.clone(),
                config: merged,
            })
        };

    // Phase 1: Collect controller-side fetch_releases jobs, deduplicated by
    // (plugin_config_id, package_identifier).
    let mut controller_job_map: HashMap<(Uuid, String), ControllerFetchJob> = HashMap::new();
    for pa in plugin_assignments
        .iter()
        .filter(|pa| pa.role == "fetch_releases")
    {
        let Some(config_model) = configs.get(&pa.plugin_config_id) else {
            continue;
        };
        let Ok(plugin_type) = serde_json::from_value::<PluginType>(serde_json::Value::String(
            config_model.plugin_type.clone(),
        )) else {
            continue;
        };
        let merged =
            uptrakit_update_hooks::merge_config(&config_model.config, pa.config_override.as_ref());
        if is_controller_fetch_site(&pa.execution_site, &plugin_type, &merged) {
            let key = (pa.plugin_config_id, pa.package_identifier.clone());
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

    // Phase 2: Execute controller-side fetches.
    let controller_checks_run = run_controller_fetch_jobs(
        tenant_db.db(),
        &state.notification_service,
        tenant_db.tenant_id,
        controller_job_map.into_values().collect(),
    )
    .await;

    // Phase 3: Send CheckVersions messages to agents for agent-side assignments.
    let mut agents_notified: u32 = 0;
    let mut seen = std::collections::HashSet::new();

    for link in &links {
        let Some(host_record) = hosts.get(&link.host_id) else {
            continue;
        };
        let Some(&service_id) = service_hosts.get(&link.host_id) else {
            continue;
        };
        if !seen.insert((service_id, link.host_id)) {
            continue;
        }

        let detect_version = plugin_by_host_role
            .get(&(link.host_id, "detect_version".to_string()))
            .and_then(|p| build_assignment(p));

        let fetch_releases = plugin_by_host_role
            .get(&(link.host_id, "fetch_releases".to_string()))
            .and_then(|p| {
                let config_model = configs.get(&p.plugin_config_id)?;
                let plugin_type: PluginType = serde_json::from_value(serde_json::Value::String(
                    config_model.plugin_type.clone(),
                ))
                .ok()?;
                let merged = uptrakit_update_hooks::merge_config(
                    &config_model.config,
                    p.config_override.as_ref(),
                );
                // Skip assignments that ran (or will run) controller-side.
                if is_controller_fetch_site(&p.execution_site, &plugin_type, &merged) {
                    None
                } else {
                    build_assignment(p)
                }
            });

        // No agent-side work for this host — controller-side fetch handled it.
        if detect_version.is_none() && fetch_releases.is_none() {
            continue;
        }

        let assignment = uptrakit_internal_wire::VersionCheckAssignment {
            software_item_id: item_id,
            name: item.name.clone(),
            detect_version,
            fetch_releases,
            host_package_id: None,
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

/// Trigger a version check for a specific software item on a specific host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/check-versions",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 200, description = "Version check triggered", body = TriggerVersionCheckResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item, host, or agent not found")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
pub async fn check_versions_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
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
    let _link = match item_queries::load_host_assignment(tenant_db.db(), host_id, item_id).await {
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
        let config = match find_raw_active_config(&tenant_db, plugin.plugin_config_id).await {
            Ok(Some(c)) => c,
            Ok(None) => continue,
            Err(e) => {
                tracing::error!(
                    plugin_config_id = %plugin.plugin_config_id,
                    error = %e,
                    "DB error loading plugin config, skipping role assignment"
                );
                continue;
            }
        };
        let Ok(plugin_type) = serde_json::from_value::<PluginType>(serde_json::Value::String(
            config.plugin_type.clone(),
        )) else {
            tracing::error!("Unknown plugin type: {}", config.plugin_type);
            continue;
        };
        let merged =
            uptrakit_update_hooks::merge_config(&config.config, plugin.config_override.as_ref());
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
        host_package_id: None,
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
