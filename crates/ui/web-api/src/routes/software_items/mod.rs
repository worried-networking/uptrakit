//! HTTP route handlers for `/api/v1/software-items`.
//!
//! Controller-side fetch orchestration lives in [`controller_fetch`].
//! Version-check context loading and agent dispatch live in [`version_check_dispatch`].

mod controller_fetch;
mod version_check_dispatch;

use crate::AppState;
use crate::actions::software_items as item_actions;
use crate::api_error::ApiError;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{
    CanCreateSoftware, CanDeleteSoftware, CanTriggerChecks, CanTriggerUpdates, CanUpdateSoftware,
    CanViewSoftware,
};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::plugin_configs::find_raw_active_config;
use crate::queries::plugin_type_settings as pts_queries;
use crate::queries::software_items as item_queries;
use crate::queries::update_triggers::TriggerUpdateParams;
use crate::queries::update_types::ActorType;
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, RelationTrait as _, Set};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host, host_software_item_plugin, prelude::*, service, service_host, software_item,
};
use uptrakit_shared_types::PluginTypeId;
use uptrakit_web_api_types::events::AdminEvent;
use uuid::Uuid;

use uptrakit_web_api_types::PluginRole;
pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, ListSoftwareItemsParams,
    MergeSoftwareItemsExecuteRequest, MergeSoftwareItemsExecuteResponse,
    MergeSoftwareItemsPreviewRequest, MergeSoftwareItemsPreviewResponse,
    SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse,
    TriggerUpdateRequest, TriggerUpdateResponse, TriggerUpdateStatus, TriggerVersionCheckResponse,
    UpdateHostAssignmentRequest, UpdateSoftwareItemRequest,
};

use controller_fetch::{ControllerFetchJob, is_controller_fetch_site, run_controller_fetch_jobs};
use version_check_dispatch::{
    collect_and_run_controller_fetches, dispatch_agent_version_checks, load_version_check_context,
};

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
    Validated(req): Validated<CreateSoftwareItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut resp = item_queries::create_software_item(&tenant_db, req).await?;

    // Fire software-item lifecycle plugins (e.g. Dashboard Icons enrichment).
    // The handler is generic — it applies whatever patch the plugins return.
    if let Some(patch) = fire_software_item_lifecycle(&state, &tenant_db, &resp).await
        && item_queries::apply_software_item_patch(tenant_db.db(), resp.id, &patch)
            .await
            .is_ok()
        && let Some(ref icon_url) = patch.icon_url
    {
        resp.icon_url = icon_url.clone();
    }

    state
        .notification
        .event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::SoftwareItemCreated { id: resp.id },
        )
        .await;
    Ok((StatusCode::CREATED, Json(resp)).into_response())
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

/// Preview a manual merge of software items.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/merge/preview",
    request_body = MergeSoftwareItemsPreviewRequest,
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 200, description = "Merge preview calculated", body = MergeSoftwareItemsPreviewResponse),
        (status = 400, description = "Invalid merge request")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn preview_software_item_merge(
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Json(req): Json<MergeSoftwareItemsPreviewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = item_queries::preview_merge_software_items(&tenant_db, &req).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Execute a manual merge of software items.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/merge/execute",
    request_body = MergeSoftwareItemsExecuteRequest,
    extensions(("x-required-permission" = json!("update_software and delete_software"))),
    responses(
        (status = 200, description = "Software items merged", body = MergeSoftwareItemsExecuteResponse),
        (status = 400, description = "Invalid merge request")
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn execute_software_item_merge(
    tenant_db: TenantDb,
    CanUpdateSoftware(_update_user): CanUpdateSoftware,
    CanDeleteSoftware(_delete_user): CanDeleteSoftware,
    Json(req): Json<MergeSoftwareItemsExecuteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = item_queries::execute_merge_software_items(&tenant_db, &req).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
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
) -> Result<impl IntoResponse, ApiError> {
    let ctx = state.mutation_context();
    let resp = item_actions::update(&tenant_db, &ctx, item_id, req).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
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
) -> Result<impl IntoResponse, ApiError> {
    if req.host_assignments.is_empty() {
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "host_assignments must not be empty",
        ));
    }

    let resp =
        item_queries::assign_hosts(state.plugin_ops.as_ref(), &tenant_db, item_id, req).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
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
) -> Result<impl IntoResponse, ApiError> {
    let resp = item_queries::update_host_assignment(
        state.plugin_ops.as_ref(),
        &tenant_db,
        item_id,
        host_id,
        req,
    )
    .await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Remove a specific plugin assignment identified by role and ordinal.
#[utoipa::path(
    delete,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/plugins/{role}/{ordinal}",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID"),
        ("role" = String, Path, description = "Plugin role (e.g. pre_update_hook)"),
        ("ordinal" = i32, Path, description = "Ordinal of the plugin assignment to remove")
    ),
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 200, description = "Plugin assignment removed", body = SoftwareItemDetailResponse),
        (status = 404, description = "Software item, host, or plugin assignment not found"),
    ),
    tag = "Software Items",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_plugin_assignment(
    tenant_db: TenantDb,
    CanUpdateSoftware(_user): CanUpdateSoftware,
    Path((item_id, host_id, role, ordinal)): Path<(Uuid, Uuid, String, i32)>,
) -> Result<impl IntoResponse, ApiError> {
    let role = match role.parse::<PluginRole>() {
        Ok(r) => r,
        Err(_) => return Ok(error_response(StatusCode::BAD_REQUEST, "invalid role")),
    };
    let resp =
        item_queries::delete_plugin_assignment(&tenant_db, item_id, host_id, role, ordinal).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
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
) -> Result<impl IntoResponse, ApiError> {
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

    let ctx = state.mutation_context();
    let user_id_str = user.user_id.to_string();
    let result = item_actions::trigger_update(
        &tenant_db,
        &ctx,
        state.controller_update_protection(),
        TriggerUpdateParams {
            tenant_id: tenant_db.tenant_id,
            item_id,
            host_id,
            to_version: req.to_version,
            actor_type: ActorType::User.as_str(),
            actor_id: &user_id_str,
            release_info,
            interactive: req.interactive,
        },
    )
    .await?;

    let status = match result.initial_status {
        uptrakit_shared_db::entity::update_history::UpdateStatus::Pending => {
            TriggerUpdateStatus::Pending
        }
        _ => TriggerUpdateStatus::Queued,
    };
    let resp = TriggerUpdateResponse {
        update_history_id: result.update_history_id,
        status,
    };
    Ok((StatusCode::OK, Json(resp)).into_response())
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

// ---------------------------------------------------------------------------
// check_versions_host helpers
// ---------------------------------------------------------------------------

/// Verify that a software item exists, a host exists and belongs to the tenant,
/// and the host is assigned to the software item.
///
/// Returns `(item, host_record, link)` on success, or an HTTP error response on
/// any failure.
async fn verify_software_item_and_host(
    tenant_db: &TenantDb,
    item_id: Uuid,
    host_id: Uuid,
) -> Result<
    (
        uptrakit_shared_db::entity::software_item::Model,
        uptrakit_shared_db::entity::host::Model,
        uptrakit_shared_db::entity::host_software_item::Model,
    ),
    Response,
> {
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
            Some(i) => i,
            None => {
                return Err(error_response(
                    StatusCode::NOT_FOUND,
                    "Software item not found",
                ));
            }
        };

    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return Err(error_response(StatusCode::NOT_FOUND, "Host not found")),
        Err(e) => {
            tracing::error!("Failed to lookup host: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let link = match item_queries::load_host_assignment(tenant_db.db(), host_id, item_id).await {
        Some(l) => l,
        None => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item",
            ));
        }
    };

    Ok((item, host_record, link))
}

/// Load the approved agent service record for a given host.
///
/// Queries `service_host` (tenant-scoped via join on `service`) and then verifies
/// the linked service is active and approved.
///
/// Returns the `service::Model` on success, or an HTTP error response on any
/// failure.
async fn load_agent_service(
    tenant_db: &TenantDb,
    host_id: Uuid,
) -> Result<uptrakit_shared_db::entity::service::Model, Response> {
    let agent_links = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(host_id))
        .all(tenant_db.db())
        .await
    {
        Ok(links) if links.is_empty() => {
            return Err(error_response(
                StatusCode::NOT_FOUND,
                "No agent linked to this host",
            ));
        }
        Ok(links) => links,
        Err(e) => {
            tracing::error!("Failed to find agent for host: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let service_ids: Vec<Uuid> = agent_links
        .into_iter()
        .map(|link| link.service_id)
        .collect();

    let agents = match Service::find()
        .filter(service::Column::Id.is_in(service_ids))
        .filter(service::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(agents) => agents,
        Err(e) => {
            tracing::error!("Failed to lookup agent: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let agent = agents
        .iter()
        .filter(|svc| svc.status == service::ServiceStatus::Approved)
        .max_by_key(|svc| svc.last_seen_at.unwrap_or(svc.updated_at))
        .cloned()
        .or_else(|| {
            agents
                .iter()
                .max_by_key(|svc| svc.last_seen_at.unwrap_or(svc.updated_at))
                .cloned()
        });

    match agent {
        Some(a) if a.status != service::ServiceStatus::Approved => Err(error_response(
            StatusCode::BAD_REQUEST,
            "Agent is not approved",
        )),
        Some(a) => Ok(a),
        None => Err(error_response(
            StatusCode::NOT_FOUND,
            "Agent not found or deactivated",
        )),
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::load_agent_service;
    use crate::tenant_db::TenantDb;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::{insert_host, link_service_host};
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_shared_db::entity::service;

    async fn insert_service_with_timestamps(
        app: &TestApp,
        id: uuid::Uuid,
        status: service::ServiceStatus,
        updated_at: time::OffsetDateTime,
        last_seen_at: Option<time::OffsetDateTime>,
    ) -> service::Model {
        service::ActiveModel {
            id: Set(id),
            tenant_id: Set(app.tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("host-{}", &id.to_string()[..8])),
            friendly_name: Set(format!("Service {}", &id.to_string()[..8])),
            ip_address: Set(Some("10.0.0.1".to_string())),
            status: Set(status),
            enrollment_secret_hash: Set(format!("secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(last_seen_at),
            created_at: Set(updated_at),
            updated_at: Set(updated_at),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(&app.db)
        .await
        .expect("insert service")
    }

    #[tokio::test]
    async fn load_agent_service_prefers_active_approved_service_when_host_has_stale_links() {
        let app = TestApp::new().await;
        let tenant_db = TenantDb::new_for_test(app.db.clone(), app.tenant_id);
        let host = insert_host(&app.db, app.tenant_id).await;

        let stale_updated_at = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        let active_updated_at = time::OffsetDateTime::now_utc();

        let stale_service = insert_service_with_timestamps(
            &app,
            uuid::Uuid::now_v7(),
            service::ServiceStatus::Approved,
            stale_updated_at,
            Some(stale_updated_at),
        )
        .await;
        let active_service = insert_service_with_timestamps(
            &app,
            uuid::Uuid::now_v7(),
            service::ServiceStatus::Approved,
            active_updated_at,
            Some(active_updated_at),
        )
        .await;

        link_service_host(&app.db, stale_service.id, host.id).await;
        link_service_host(&app.db, active_service.id, host.id).await;

        service::ActiveModel {
            id: Set(stale_service.id),
            deactivated_at: Set(Some(time::OffsetDateTime::now_utc())),
            ..stale_service.into()
        }
        .update(&app.db)
        .await
        .expect("deactivate stale service");

        let agent = load_agent_service(&tenant_db, host.id)
            .await
            .expect("should select active approved service");

        assert_eq!(agent.id, active_service.id);
    }
}

/// Classify plugin rows into controller-side fetch jobs and agent-side
/// `detect_version` / `fetch_releases` assignments.
///
/// Returns `(controller_fetch_jobs, detect_version, fetch_releases)`.
async fn classify_role_assignments(
    tenant_db: &TenantDb,
    plugin_rows: &[uptrakit_shared_db::entity::host_software_item_plugin::Model],
    host_id: Uuid,
    item_id: Uuid,
) -> (
    Vec<ControllerFetchJob>,
    Option<uptrakit_internal_wire::PluginAssignment>,
    Option<uptrakit_internal_wire::PluginAssignment>,
) {
    let mut detect_version: Option<uptrakit_internal_wire::PluginAssignment> = None;
    let mut fetch_releases: Option<uptrakit_internal_wire::PluginAssignment> = None;
    let mut controller_fetch_jobs: Vec<ControllerFetchJob> = Vec::new();

    for plugin in plugin_rows {
        let config = match plugin.plugin_config_id {
            Some(pc_id) => match find_raw_active_config(tenant_db, pc_id).await {
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
        let plugin_type = PluginTypeId::new(plugin_type_str);
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

    (controller_fetch_jobs, detect_version, fetch_releases)
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
    // Phase 1–3: verify software item, host, and assignment.
    let (item, host_record, link) =
        match verify_software_item_and_host(&tenant_db, item_id, host_id).await {
            Ok(t) => t,
            Err(resp) => return resp,
        };

    // Phase 4–5: load approved agent service for this host.
    let agent = match load_agent_service(&tenant_db, host_id).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    // Phase 6: load role-specific plugin assignments for this host.
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

    // Phase 7: classify plugins into controller jobs vs agent assignments.
    let (controller_fetch_jobs, detect_version, fetch_releases) =
        classify_role_assignments(&tenant_db, &role_plugins, host_id, item_id).await;

    // Phase 8a: run controller-side fetch_releases (e.g. GitHub, Docker).
    let controller_checks_run = run_controller_fetch_jobs(
        tenant_db.db(),
        &state.notification.notification_service,
        &state.notification.event_broadcaster,
        tenant_db.tenant_id,
        controller_fetch_jobs,
    )
    .await;

    // Phase 8b: if no agent-side work is needed, return immediately.
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

    // Phase 8c: dispatch CheckVersions to the agent.
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
    state
        .notification
        .notification_service
        .send(&agent.id, msg)
        .await;

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
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let ctx = state.mutation_context();
    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => match item_actions::batch_feature(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch approve failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "delete" => match item_actions::batch_delete(&tenant_db, &ctx, &body.ids).await {
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

// ---------------------------------------------------------------------------
// Software-item lifecycle plugin dispatch
// ---------------------------------------------------------------------------

/// Fire `on_software_item_created` lifecycle hooks for newly created items.
///
/// Returns the merged patch from all responding plugins, or `None` when no
/// plugin returned a patch.
async fn fire_software_item_lifecycle(
    state: &AppState,
    tenant_db: &TenantDb,
    resp: &SoftwareItemResponse,
) -> Option<uptrakit_plugin_infrastructure_registry::SoftwareItemPatch> {
    let event = uptrakit_plugin_infrastructure_registry::SoftwareItemCreatedEvent::new(
        resp.id,
        tenant_db.tenant_id,
        resp.name.clone(),
        resp.featured,
        resp.icon_url.clone(),
    );

    let lifecycle_ctx = match pts_queries::preload_lifecycle_type_settings(
        tenant_db.db(),
        tenant_db.tenant_id,
        state.plugin_ops.as_ref(),
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!(
                error = %e,
                tenant_id = %tenant_db.tenant_id,
                "failed to preload lifecycle type settings; using defaults"
            );
            uptrakit_plugin_infrastructure_registry::SoftwareItemLifecycleContext::default()
        }
    };

    state
        .plugin_ops
        .on_software_item_created(&event, &lifecycle_ctx)
        .await
}
