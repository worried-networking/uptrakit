//! HTTP route handlers for `/api/v1/software-items`.
//!
//! Controller-side fetch orchestration lives in [`controller_fetch`].
//! Version-check context loading and agent dispatch live in [`version_check_dispatch`].

mod controller_fetch;
mod version_check_dispatch;

use crate::AppState;
use crate::actions::software_items as item_actions;
use crate::api_error::ApiError;
use crate::app_state::AuditEmitterState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{
    CanCreateSoftware, CanDeleteSoftware, CanTriggerChecks, CanTriggerUpdates, CanUpdateSoftware,
    CanViewSoftware,
};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::plugin_configs::find_raw_active_config;
use crate::queries::plugin_type_settings as pts_queries;
use crate::queries::software_items as item_queries;
use crate::queries::update_triggers::TriggerUpdateParams;
use crate::queries::update_types::ActorType;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
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

const SOFTWARE_ITEM_CREATE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_CREATE;
const SOFTWARE_ITEM_UPDATE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE;
const SOFTWARE_ITEM_DELETE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_DELETE;
const SOFTWARE_ITEM_APPROVE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_APPROVE;
const SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_ASSIGN_HOSTS;
const SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UNASSIGN_HOST;
const SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT;
const SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION:
    uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT;
const SOFTWARE_ITEM_MERGE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_MERGE;
const SOFTWARE_ITEM_BATCH_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_BATCH;
const SOFTWARE_VERSION_CHECK_TRIGGERED_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED;

struct AuditContext<'a> {
    audit_emitter: &'a uptrakit_audit_log::AuditEmitter,
    tenant_id: Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_software_item_mutation_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_id: String,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);
    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .target("software_item", target_id, target_display)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_best_effort(entry);
    }
}

fn emit_software_update_audit(
    state: &AppState,
    tenant_id: Uuid,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    item_id: Uuid,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);
    let entry = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .target("software_item", item_id.to_string(), None)
    .outcome(outcome)
    .details(details)
    .build();

    if let Ok(entry) = entry {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn emit_software_version_check_audit(
    ctx: &AuditContext<'_>,
    item_id: Uuid,
    item_name: Option<&str>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);
    let entry =
        uptrakit_audit_log::AuditEntry::builder(SOFTWARE_VERSION_CHECK_TRIGGERED_AUDIT_ACTION)
            .tenant_scope(ctx.tenant_id)
            .actor(actor_type, actor_id)
            .target(
                "software_item",
                item_id.to_string(),
                item_name.map(str::to_string),
            )
            .outcome(outcome)
            .details(details)
            .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_best_effort(entry);
    }
}

fn version_check_dispatch_mode(agents_notified: u32, controller_checks_run: u32) -> &'static str {
    match (agents_notified > 0, controller_checks_run > 0) {
        (true, true) => "hybrid",
        (true, false) => "agent_only",
        (false, true) => "controller_only",
        (false, false) => "none",
    }
}

fn classify_version_check_context_load_failure(
    status: StatusCode,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match status {
        StatusCode::NOT_FOUND => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "version_check.no_hosts_assigned",
        ),
        _ => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "version_check.internal_error",
        ),
    }
}

fn classify_trigger_update_dispatch_audit_outcome(
    status: uptrakit_shared_db::entity::update_history::UpdateStatus,
) -> uptrakit_audit_log::AuditOutcome {
    match status {
        uptrakit_shared_db::entity::update_history::UpdateStatus::Failed => {
            uptrakit_audit_log::AuditOutcome::Failed
        }
        _ => uptrakit_audit_log::AuditOutcome::Success,
    }
}

#[cfg(test)]
mod trigger_update_audit_outcome_tests {
    use super::classify_trigger_update_dispatch_audit_outcome;
    use uptrakit_shared_db::entity::update_history;

    #[test]
    fn failed_dispatch_status_maps_to_failed_audit_outcome() {
        assert_eq!(
            classify_trigger_update_dispatch_audit_outcome(update_history::UpdateStatus::Failed),
            uptrakit_audit_log::AuditOutcome::Failed
        );
    }

    #[test]
    fn non_failed_dispatch_status_maps_to_success_audit_outcome() {
        assert_eq!(
            classify_trigger_update_dispatch_audit_outcome(update_history::UpdateStatus::Pending),
            uptrakit_audit_log::AuditOutcome::Success
        );
        assert_eq!(
            classify_trigger_update_dispatch_audit_outcome(update_history::UpdateStatus::Queued),
            uptrakit_audit_log::AuditOutcome::Success
        );
    }
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
    CanCreateSoftware(user): CanCreateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreateSoftwareItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let mut resp = match item_queries::create_software_item(&tenant_db, req).await {
        Ok(resp) => resp,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_CREATE_AUDIT_ACTION,
                "pending".to_string(),
                None,
                outcome,
                serde_json::json!({
                    "reason_code": reason_code,
                }),
            );
            return Err(err.into());
        }
    };

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

    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_CREATE_AUDIT_ACTION,
        resp.id.to_string(),
        Some(resp.name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "featured": resp.featured,
            "has_icon_url": resp.icon_url.is_some(),
        }),
    );

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
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanUpdateSoftware(update_user): CanUpdateSoftware,
    CanDeleteSoftware(_delete_user): CanDeleteSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<MergeSoftwareItemsExecuteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id,
        user: &update_user,
        api_token_id,
    };
    let requested_count = req.candidate_ids.len();
    let resp = match item_queries::execute_merge_software_items(&tenant_db, &req).await {
        Ok(resp) => resp,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_MERGE_AUDIT_ACTION,
                req.survivor_id.to_string(),
                None,
                outcome,
                serde_json::json!({
                    "reason_code": reason_code,
                    "candidate_count": requested_count,
                }),
            );
            return Err(err.into());
        }
    };

    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_MERGE_AUDIT_ACTION,
        resp.survivor_id.to_string(),
        None,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "candidate_count": requested_count,
            "deleted_count": resp.deleted_ids.len(),
            "moved_link_count": resp.moved_link_ids.len(),
            "skipped_duplicate_link_count": resp.skipped_duplicate_link_ids.len(),
        }),
    );

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
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<UpdateSoftwareItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = state.mutation_context();
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let name_changed = req.name.is_some();
    let featured_changed = req.featured.is_some();
    let icon_url_changed = !req.icon_url.is_keep();
    let resp = match item_actions::update(&tenant_db, &ctx, item_id, req).await {
        Ok(resp) => resp,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_UPDATE_AUDIT_ACTION,
                item_id.to_string(),
                None,
                outcome,
                serde_json::json!({
                    "reason_code": reason_code,
                }),
            );
            return Err(err.into());
        }
    };

    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_UPDATE_AUDIT_ACTION,
        item_id.to_string(),
        Some(resp.name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "changed_fields": {
                "name": name_changed,
                "featured": featured_changed,
                "icon_url": icon_url_changed,
            }
        }),
    );

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
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanDeleteSoftware(user): CanDeleteSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    match item_queries::delete_software_item(&tenant_db, item_id).await {
        Ok(true) => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_DELETE_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({}),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_DELETE_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "software_item.not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Software item not found")
        }
        Err(e) => {
            tracing::error!("Failed to delete software item: {e}");
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_DELETE_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_item.database_error",
                }),
            );
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
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let item = match software_item::Entity::find_by_id(item_id)
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(i)) => i,
        Ok(None) => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "software_item.not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Software item not found");
        }
        Err(e) => {
            tracing::error!("Failed to fetch software item: {e}");
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_item.database_error",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if item.featured {
        emit_software_item_mutation_audit(
            &audit_ctx,
            SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
            item_id.to_string(),
            Some(item.name.clone()),
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "software_item.already_featured",
            }),
        );
        return error_response(StatusCode::CONFLICT, "Software item is already featured");
    }

    let now = OffsetDateTime::now_utc();
    let item_name = item.name.clone();
    let mut active: software_item::ActiveModel = item.into();
    active.featured = Set(true);
    active.updated_at = Set(now);

    let updated = match active.update(tenant_db.db()).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to approve software item: {e}");
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
                item_id.to_string(),
                Some(item_name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_item.database_error",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match item_queries::get_software_item(&tenant_db, updated.id).await {
        Ok(Some(resp)) => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
                item_id.to_string(),
                Some(resp.name.clone()),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "featured": true,
                }),
            );
            (StatusCode::OK, Json(resp)).into_response()
        }
        Ok(None) => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_item.post_update_not_found",
                }),
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Item not found after update",
            )
        }
        Err(e) => {
            tracing::error!("Failed to fetch approved software item: {e}");
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "software_item.database_error",
                }),
            );
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
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<AssignHostsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    if req.host_assignments.is_empty() {
        emit_software_item_mutation_audit(
            &audit_ctx,
            SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION,
            item_id.to_string(),
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "software_item.host_assignments_empty",
            }),
        );
        return Ok(error_response(
            StatusCode::BAD_REQUEST,
            "host_assignments must not be empty",
        ));
    }

    let assignment_count = req.host_assignments.len();
    let resp = match item_queries::assign_hosts(state.plugin_ops.as_ref(), &tenant_db, item_id, req)
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION,
                item_id.to_string(),
                None,
                outcome,
                serde_json::json!({
                    "reason_code": reason_code,
                }),
            );
            return Err(err.into());
        }
    };

    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION,
        item_id.to_string(),
        Some(resp.name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "assigned_hosts_count": assignment_count,
            "host_count_after": resp.host_count,
        }),
    );

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
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<DeleteHostAssignmentParams>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let ignore_requested = params.ignore.unwrap_or(false);
    // If ignore=true, load the software item name before deleting so we can
    // create a name-based ignore rule.
    let ignore_name: Option<String> = if ignore_requested {
        match SoftwareItem::find_by_id(item_id)
            .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
            .one(tenant_db.db())
            .await
        {
            Ok(Some(item)) => Some(item.name),
            Ok(None) => {
                emit_software_item_mutation_audit(
                    &audit_ctx,
                    SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
                    item_id.to_string(),
                    None,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    serde_json::json!({
                        "host_id": host_id,
                        "reason_code": "software_item.not_found",
                    }),
                );
                return error_response(StatusCode::NOT_FOUND, "Software item not found");
            }
            Err(e) => {
                tracing::error!("Failed to look up software item for ignore: {e}");
                emit_software_item_mutation_audit(
                    &audit_ctx,
                    SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
                    item_id.to_string(),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "host_id": host_id,
                        "reason_code": "software_item.database_error",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    } else {
        None
    };

    match item_queries::unassign_host(&tenant_db, item_id, host_id).await {
        Ok(true) => {
            let mut ignore_rule_created = false;
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
            } else if ignore_requested {
                ignore_rule_created = true;
            }
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "host_id": host_id,
                    "ignore_requested": ignore_requested,
                    "ignore_rule_created": ignore_rule_created,
                }),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "host_id": host_id,
                    "reason_code": "software_item.assignment_not_found",
                }),
            );
            error_response(
                StatusCode::NOT_FOUND,
                "Software item or host assignment not found",
            )
        }
        Err(e) => {
            tracing::error!("Failed to unassign host from software item: {e}");
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "host_id": host_id,
                    "reason_code": "software_item.database_error",
                }),
            );
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
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateHostAssignmentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let role = req.role.as_str().to_string();
    let ordinal = req.ordinal;
    let resp = match item_queries::update_host_assignment(
        state.plugin_ops.as_ref(),
        &tenant_db,
        item_id,
        host_id,
        req,
    )
    .await
    {
        Ok(resp) => resp,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION,
                item_id.to_string(),
                None,
                outcome,
                serde_json::json!({
                    "host_id": host_id,
                    "role": role,
                    "ordinal": ordinal,
                    "reason_code": reason_code,
                }),
            );
            return Err(err.into());
        }
    };

    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION,
        item_id.to_string(),
        Some(resp.name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "host_id": host_id,
            "role": role,
            "ordinal": ordinal,
        }),
    );

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
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id, role, ordinal)): Path<(Uuid, Uuid, String, i32)>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let requested_role = role.clone();
    let role = match role.parse::<PluginRole>() {
        Ok(r) => r,
        Err(_) => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
                item_id.to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "host_id": host_id,
                    "role": requested_role,
                    "ordinal": ordinal,
                    "reason_code": "software_item.invalid_role",
                }),
            );
            return Ok(error_response(StatusCode::BAD_REQUEST, "invalid role"));
        }
    };

    let role_str = role.as_str().to_string();
    let resp =
        match item_queries::delete_plugin_assignment(&tenant_db, item_id, host_id, role, ordinal)
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                let (outcome, reason_code) = err.current_context().audit_classification();
                emit_software_item_mutation_audit(
                    &audit_ctx,
                    SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
                    item_id.to_string(),
                    None,
                    outcome,
                    serde_json::json!({
                        "host_id": host_id,
                        "role": role_str,
                        "ordinal": ordinal,
                        "reason_code": reason_code,
                    }),
                );
                return Err(err.into());
            }
        };

    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
        item_id.to_string(),
        Some(resp.name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "host_id": host_id,
            "role": role_str,
            "ordinal": ordinal,
        }),
    );

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
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<TriggerUpdateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (update_actor_type, update_actor_id) = match api_token_id {
        Some(token_id) => (ActorType::ApiToken, token_id.0.to_string()),
        None => (ActorType::User, user.user_id.to_string()),
    };
    let to_version = req.to_version.clone();
    let interactive = req.interactive;
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
    let result = match item_actions::trigger_update(
        &tenant_db,
        &ctx,
        state.controller_update_protection(),
        TriggerUpdateParams {
            tenant_id: tenant_db.tenant_id,
            item_id,
            host_id,
            to_version: to_version.clone(),
            actor_type: update_actor_type.as_str(),
            actor_id: &update_actor_id,
            release_info,
            interactive,
        },
    )
    .await
    {
        Ok(result) => result,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().trigger_audit_classification();
            emit_software_update_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                item_id,
                outcome,
                serde_json::json!({
                    "host_id": host_id,
                    "to_version": to_version,
                    "interactive": interactive,
                    "reason_code": reason_code,
                }),
            );
            return Err(err.into());
        }
    };

    let status = match result.initial_status {
        uptrakit_shared_db::entity::update_history::UpdateStatus::Pending => {
            TriggerUpdateStatus::Pending
        }
        uptrakit_shared_db::entity::update_history::UpdateStatus::Failed => {
            TriggerUpdateStatus::Failed
        }
        _ => TriggerUpdateStatus::Queued,
    };
    let resp = TriggerUpdateResponse {
        update_history_id: result.update_history_id,
        status,
    };

    emit_software_update_audit(
        &state,
        tenant_db.tenant_id,
        &user,
        api_token_id,
        item_id,
        classify_trigger_update_dispatch_audit_outcome(result.initial_status),
        serde_json::json!({
            "host_id": host_id,
            "to_version": to_version,
            "interactive": interactive,
            "update_history_id": result.update_history_id,
            "dispatch_status": resp.status.to_string(),
        }),
    );

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
    CanTriggerChecks(user): CanTriggerChecks,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };

    // Verify software item exists and is active
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
            Some(i) => i,
            None => {
                emit_software_version_check_audit(
                    &audit_ctx,
                    item_id,
                    None,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    serde_json::json!({
                        "dispatch_scope": "all_hosts",
                        "reason_code": "version_check.software_item_not_found",
                    }),
                );
                return error_response(StatusCode::NOT_FOUND, "Software item not found");
            }
        };

    // Phase 1: Load all data needed for version checks.
    let ctx = match load_version_check_context(&tenant_db, item_id).await {
        Ok(ctx) => ctx,
        Err(resp) => {
            let (outcome, reason_code) = classify_version_check_context_load_failure(resp.status());
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                Some(&item.name),
                outcome,
                serde_json::json!({
                    "dispatch_scope": "all_hosts",
                    "reason_code": reason_code,
                }),
            );
            return resp;
        }
    };

    // Phase 2: Collect and run controller-side fetch_releases jobs.
    let controller_checks_run = collect_and_run_controller_fetches(&tenant_db, &state, &ctx).await;

    // Phase 3: Send CheckVersions messages to agents for agent-side assignments.
    let agents_notified = dispatch_agent_version_checks(&state, &ctx, item_id, &item.name).await;

    if agents_notified == 0 && controller_checks_run == 0 {
        emit_software_version_check_audit(
            &audit_ctx,
            item_id,
            Some(&item.name),
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "agents_notified": agents_notified,
                "controller_checks_run": controller_checks_run,
                "dispatch_mode": version_check_dispatch_mode(agents_notified, controller_checks_run),
                "dispatch_scope": "all_hosts",
                "reason_code": "version_check.no_approved_agents",
            }),
        );
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
    emit_software_version_check_audit(
        &audit_ctx,
        item_id,
        Some(&item.name),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "agents_notified": agents_notified,
            "controller_checks_run": controller_checks_run,
            "dispatch_mode": version_check_dispatch_mode(agents_notified, controller_checks_run),
            "dispatch_scope": "all_hosts",
        }),
    );
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
#[derive(Debug)]
enum CheckVersionsHostPreconditionError {
    SoftwareItemNotFound,
    HostNotFound,
    HostNotAssigned,
    Internal,
}

impl CheckVersionsHostPreconditionError {
    fn into_response(self) -> Response {
        match self {
            Self::SoftwareItemNotFound => {
                error_response(StatusCode::NOT_FOUND, "Software item not found")
            }
            Self::HostNotFound => error_response(StatusCode::NOT_FOUND, "Host not found"),
            Self::HostNotAssigned => error_response(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item",
            ),
            Self::Internal => {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }

    fn audit(&self) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::SoftwareItemNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "version_check.software_item_not_found",
            ),
            Self::HostNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "version_check.host_not_found",
            ),
            Self::HostNotAssigned => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "version_check.host_not_assigned",
            ),
            Self::Internal => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "version_check.internal_error",
            ),
        }
    }
}

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
    CheckVersionsHostPreconditionError,
> {
    let item =
        match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id, item_id).await {
            Some(i) => i,
            None => return Err(CheckVersionsHostPreconditionError::SoftwareItemNotFound),
        };

    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return Err(CheckVersionsHostPreconditionError::HostNotFound),
        Err(e) => {
            tracing::error!("Failed to lookup host: {e}");
            return Err(CheckVersionsHostPreconditionError::Internal);
        }
    };

    let link = match item_queries::load_host_assignment(tenant_db.db(), host_id, item_id).await {
        Some(l) => l,
        None => return Err(CheckVersionsHostPreconditionError::HostNotAssigned),
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
#[derive(Debug)]
enum LoadAgentServiceError {
    NoAgentLinked,
    AgentNotApproved,
    AgentNotFound,
    Internal,
}

impl LoadAgentServiceError {
    fn into_response(self) -> Response {
        match self {
            Self::NoAgentLinked => {
                error_response(StatusCode::NOT_FOUND, "No agent linked to this host")
            }
            Self::AgentNotApproved => {
                error_response(StatusCode::BAD_REQUEST, "Agent is not approved")
            }
            Self::AgentNotFound => {
                error_response(StatusCode::NOT_FOUND, "Agent not found or deactivated")
            }
            Self::Internal => {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }

    fn audit(&self) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::NoAgentLinked => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "version_check.no_agent_linked",
            ),
            Self::AgentNotApproved => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "version_check.agent_not_approved",
            ),
            Self::AgentNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "version_check.agent_not_found",
            ),
            Self::Internal => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "version_check.internal_error",
            ),
        }
    }
}

async fn load_agent_service(
    tenant_db: &TenantDb,
    host_id: Uuid,
) -> Result<uptrakit_shared_db::entity::service::Model, LoadAgentServiceError> {
    let agent_links = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(host_id))
        .all(tenant_db.db())
        .await
    {
        Ok(links) if links.is_empty() => return Err(LoadAgentServiceError::NoAgentLinked),
        Ok(links) => links,
        Err(e) => {
            tracing::error!("Failed to find agent for host: {e}");
            return Err(LoadAgentServiceError::Internal);
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
            return Err(LoadAgentServiceError::Internal);
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
        Some(a) if a.status != service::ServiceStatus::Approved => {
            Err(LoadAgentServiceError::AgentNotApproved)
        }
        Some(a) => Ok(a),
        None => Err(LoadAgentServiceError::AgentNotFound),
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod audit_tests {
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
) -> Result<
    (
        Vec<ControllerFetchJob>,
        Option<uptrakit_internal_wire::PluginAssignment>,
        Option<uptrakit_internal_wire::PluginAssignment>,
    ),
    Response,
> {
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

    Ok((controller_fetch_jobs, detect_version, fetch_releases))
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
    CanTriggerChecks(user): CanTriggerChecks,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };

    // Phase 1–3: verify software item, host, and assignment.
    let (item, host_record, link) =
        match verify_software_item_and_host(&tenant_db, item_id, host_id).await {
            Ok(t) => t,
            Err(error) => {
                let (outcome, reason_code) = error.audit();
                emit_software_version_check_audit(
                    &audit_ctx,
                    item_id,
                    None,
                    outcome,
                    serde_json::json!({
                        "dispatch_scope": "single_host",
                        "host_id": host_id,
                        "reason_code": reason_code,
                    }),
                );
                return error.into_response();
            }
        };

    // Phase 4–5: load approved agent service for this host.
    let agent = match load_agent_service(&tenant_db, host_id).await {
        Ok(a) => a,
        Err(error) => {
            let (outcome, reason_code) = error.audit();
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                Some(&item.name),
                outcome,
                serde_json::json!({
                    "dispatch_scope": "single_host",
                    "host_id": host_id,
                    "reason_code": reason_code,
                }),
            );
            return error.into_response();
        }
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
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                Some(&item.name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "dispatch_scope": "single_host",
                    "host_id": host_id,
                    "reason_code": "version_check.internal_error",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Phase 7: classify plugins into controller jobs vs agent assignments.
    let (controller_fetch_jobs, detect_version, fetch_releases) =
        match classify_role_assignments(&tenant_db, &role_plugins, host_id, item_id).await {
            Ok(result) => result,
            Err(resp) => return resp,
        };

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
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                Some(&item.name),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "agents_notified": 0,
                    "controller_checks_run": controller_checks_run,
                    "dispatch_mode": version_check_dispatch_mode(0, controller_checks_run),
                    "dispatch_scope": "single_host",
                    "host_id": host_id,
                }),
            );
            return (StatusCode::OK, Json(resp)).into_response();
        }
        emit_software_version_check_audit(
            &audit_ctx,
            item_id,
            Some(&item.name),
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "agents_notified": 0,
                "controller_checks_run": controller_checks_run,
                "dispatch_mode": version_check_dispatch_mode(0, controller_checks_run),
                "dispatch_scope": "single_host",
                "host_id": host_id,
                "reason_code": "version_check.no_role_assignments",
            }),
        );
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
    emit_software_version_check_audit(
        &audit_ctx,
        item_id,
        Some(&item.name),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "agents_notified": 1,
            "controller_checks_run": controller_checks_run,
            "dispatch_mode": version_check_dispatch_mode(1, controller_checks_run),
            "dispatch_scope": "single_host",
            "host_id": host_id,
        }),
    );

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
    CanDeleteSoftware(user): CanDeleteSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let ctx = state.mutation_context();
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let requested_count = body.ids.len();

    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => match item_actions::batch_feature(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch approve failed: {e}");
                emit_software_item_mutation_audit(
                    &audit_ctx,
                    SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
                    "batch".to_string(),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "action": body.action,
                        "requested_count": requested_count,
                        "reason_code": "software_item.batch_approve_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "delete" => match item_actions::batch_delete(&tenant_db, &ctx, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch delete failed: {e}");
                emit_software_item_mutation_audit(
                    &audit_ctx,
                    SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
                    "batch".to_string(),
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "action": body.action,
                        "requested_count": requested_count,
                        "reason_code": "software_item.batch_delete_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        unknown => {
            emit_software_item_mutation_audit(
                &audit_ctx,
                SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
                "batch".to_string(),
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "action": unknown,
                    "requested_count": requested_count,
                    "reason_code": "software_item.batch_unknown_action",
                }),
            );
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

    let outcome = if response.failed.is_empty() {
        uptrakit_audit_log::AuditOutcome::Success
    } else {
        uptrakit_audit_log::AuditOutcome::Partial
    };
    emit_software_item_mutation_audit(
        &audit_ctx,
        SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
        "batch".to_string(),
        None,
        outcome,
        serde_json::json!({
            "action": body.action,
            "requested_count": requested_count,
            "succeeded_count": response.succeeded.len(),
            "failed_count": response.failed.len(),
        }),
    );

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

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::middleware::permission::{
        CanCreateSoftware, CanDeleteSoftware, CanTriggerUpdates, CanUpdateSoftware,
    };
    use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
    use crate::test_harness::{
        build_test_state_with_plugin_ops, insert_default_tenant, setup_migrated_db,
    };
    use async_trait::async_trait;
    use sea_orm::{
        ActiveModelTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
    };
    use std::pin::Pin;
    use uptrakit_plugin_infrastructure_registry::{
        CatalogConfig, ControllerPostUpdateContext, ControllerProtectionContext,
        ControllerProtectionDecision, ControllerUpdateProtection, ControllerUpdateProtectionOps,
        NotificationOps, NotificationTransport, PluginConfigOps, PluginMetadataOps, PluginOps,
        PluginSurfaceActionOps, PluginSurfaceOps, PostUpdateOutcome, SoftwareItemCreatedEvent,
        SoftwareItemLifecycle, SoftwareItemLifecycleContext, SoftwareItemLifecycleOps,
        SoftwareItemPatch, build_catalog,
    };
    use uptrakit_shared_db::entity::{
        audit_log, host_software_item, host_software_item_plugin, software_item, update_history,
    };

    struct SkipProtection;

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for SkipProtection {
        fn plugin_type_id(&self) -> uptrakit_shared_types::PluginTypeId {
            uptrakit_shared_types::PluginTypeId::from_static("test_skip_protection")
        }
    }

    #[async_trait]
    impl ControllerUpdateProtection for SkipProtection {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> uptrakit_plugin_infrastructure_registry::PluginResult<ControllerProtectionDecision>
        {
            Ok(ControllerProtectionDecision::skipped(Some(
                "test skipped protection".to_string(),
            )))
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> uptrakit_plugin_infrastructure_registry::PluginResult<PostUpdateOutcome> {
            Ok(PostUpdateOutcome::default())
        }
    }

    struct ProtectionOverridePluginOps {
        inner: Arc<dyn PluginOps>,
        protection: Arc<dyn ControllerUpdateProtection>,
    }

    impl PluginMetadataOps for ProtectionOverridePluginOps {
        fn get(
            &self,
            id: &uptrakit_shared_types::PluginTypeId,
        ) -> Option<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
            self.inner.get(id)
        }

        fn all(&self) -> Vec<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
            self.inner.all()
        }
    }

    impl PluginConfigOps for ProtectionOverridePluginOps {}

    impl PluginSurfaceActionOps for ProtectionOverridePluginOps {
        fn handle_surface_action<'a>(
            &'a self,
            ctx: &'a uptrakit_plugin_infrastructure_registry::SurfaceActionContext<'a>,
            surface_id: &'a str,
            action_id: &'a str,
            params: serde_json::Value,
        ) -> Pin<
            Box<
                dyn std::future::Future<
                        Output = std::result::Result<
                            serde_json::Value,
                            uptrakit_plugin_infrastructure_registry::SurfaceActionError,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            self.inner
                .handle_surface_action(ctx, surface_id, action_id, params)
        }
    }

    impl PluginSurfaceOps for ProtectionOverridePluginOps {
        fn surface_registrations(
            &self,
        ) -> Vec<uptrakit_internal_wire::surfaces::SurfaceRegistration> {
            self.inner.surface_registrations()
        }
    }

    impl NotificationOps for ProtectionOverridePluginOps {
        fn transport(
            &self,
            id: &uptrakit_shared_types::PluginTypeId,
        ) -> Option<Arc<dyn NotificationTransport>> {
            self.inner.transport(id)
        }

        fn notification_supported_types(&self) -> Vec<uptrakit_shared_types::PluginTypeId> {
            self.inner.notification_supported_types()
        }
    }

    impl SoftwareItemLifecycleOps for ProtectionOverridePluginOps {
        fn on_software_item_created<'a>(
            &'a self,
            event: &'a SoftwareItemCreatedEvent,
            ctx: &'a SoftwareItemLifecycleContext,
        ) -> Pin<Box<dyn std::future::Future<Output = Option<SoftwareItemPatch>> + Send + 'a>>
        {
            self.inner.on_software_item_created(event, ctx)
        }

        fn software_item_lifecycle_plugins(&self) -> &[Arc<dyn SoftwareItemLifecycle>] {
            self.inner.software_item_lifecycle_plugins()
        }
    }

    impl ControllerUpdateProtectionOps for ProtectionOverridePluginOps {
        fn controller_update_protection(&self) -> Option<Arc<dyn ControllerUpdateProtection>> {
            Some(self.protection.clone())
        }
    }

    async fn build_test_state_without_real_protection(
        db: DatabaseConnection,
        tenant_id: Uuid,
    ) -> Arc<AppState> {
        let base_plugin_ops: Arc<dyn PluginOps> = Arc::new(
            build_catalog(&CatalogConfig::default()).expect("catalog should build in tests"),
        );
        let plugin_ops: Arc<dyn PluginOps> = Arc::new(ProtectionOverridePluginOps {
            inner: base_plugin_ops,
            protection: Arc::new(SkipProtection),
        });
        let (state, _jwt) = build_test_state_with_plugin_ops(db, tenant_id, Some(plugin_ops)).await;
        state
    }

    async fn insert_software_item_row(db: &DatabaseConnection, tenant_id: Uuid, item_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("Audit App".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software item");
    }

    async fn insert_software_item_row_with_flags(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        item_id: Uuid,
        name: &str,
        featured: bool,
    ) {
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            featured: Set(featured),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software item with flags");
    }

    async fn insert_host_assignment(db: &DatabaseConnection, host_id: Uuid, item_id: Uuid) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let host_software_item_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(host_software_item_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(Some("1.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host assignment");
        host_software_item_id
    }

    async fn insert_execute_update_plugin(
        db: &DatabaseConnection,
        host_id: Uuid,
        item_id: Uuid,
        host_software_item_id: Uuid,
    ) {
        let now = OffsetDateTime::now_utc();
        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(None),
            plugin_type: Set("package_manager_apt".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("pkg".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert execute_update plugin");
    }

    async fn insert_detect_version_plugin(
        db: &DatabaseConnection,
        host_id: Uuid,
        item_id: Uuid,
        host_software_item_id: Uuid,
        execution_site: &str,
    ) {
        let now = OffsetDateTime::now_utc();
        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(None),
            plugin_type: Set("package_manager_apt".to_string()),
            role: Set("detect_version".to_string()),
            ordinal: Set(0),
            package_identifier: Set("pkg".to_string()),
            config: Set(None),
            execution_site: Set(execution_site.to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert detect_version plugin");
    }

    async fn tenant_audit_row_for_action(
        db: &DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row");
    }

    async fn tenant_audit_row_for_action_and_outcome(
        db: &DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
        outcome: &'static str,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .filter(audit_log::Column::Outcome.eq(outcome))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row with outcome");
    }

    fn auth_user_with(permission: Permission) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![permission],
        }
    }

    #[tokio::test]
    async fn create_software_item_writes_success_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = match create_software_item(
            State(Arc::clone(&state)),
            tenant_db,
            CanCreateSoftware::new(auth_user_with(Permission::CreateSoftware)),
            None,
            Validated(CreateSoftwareItemRequest {
                name: "Create Audit App".to_string(),
                featured: true,
                icon_url: None,
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(err) => panic!(
                "create software item should succeed, got status {}",
                err.into_response().status()
            ),
        };
        assert_eq!(response.status(), StatusCode::CREATED);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_CREATE_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        let details = row.details_json.expect("details");
        assert_eq!(details["featured"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn create_software_item_duplicate_writes_validation_failed_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let req = CreateSoftwareItemRequest {
            name: "Duplicate Create App".to_string(),
            featured: false,
            icon_url: None,
        };

        let first = match create_software_item(
            State(Arc::clone(&state)),
            tenant_db,
            CanCreateSoftware::new(auth_user_with(Permission::CreateSoftware)),
            None,
            Validated(CreateSoftwareItemRequest {
                name: req.name.clone(),
                featured: req.featured,
                icon_url: req.icon_url.clone(),
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(err) => panic!(
                "first create should succeed, got status {}",
                err.into_response().status()
            ),
        };
        assert_eq!(first.status(), StatusCode::CREATED);

        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let err = match create_software_item(
            State(Arc::clone(&state)),
            tenant_db,
            CanCreateSoftware::new(auth_user_with(Permission::CreateSoftware)),
            None,
            Validated(req),
        )
        .await
        {
            Ok(response) => panic!(
                "duplicate create must fail, got status {}",
                response.into_response().status()
            ),
            Err(err) => err,
        };
        assert_eq!(err.into_response().status(), StatusCode::CONFLICT);

        let row = tenant_audit_row_for_action_and_outcome(
            &db,
            SOFTWARE_ITEM_CREATE_AUDIT_ACTION,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str(),
        )
        .await;
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_item.duplicate_item")
        );
    }

    #[tokio::test]
    async fn update_software_item_missing_item_writes_denied_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let missing_item_id = Uuid::now_v7();

        let err = match update_software_item(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateSoftware::new(auth_user_with(Permission::UpdateSoftware)),
            None,
            Path(missing_item_id),
            Json(UpdateSoftwareItemRequest {
                name: Some("Nope".to_string()),
                featured: None,
                icon_url: Default::default(),
            }),
        )
        .await
        {
            Ok(response) => panic!(
                "update should fail, got status {}",
                response.into_response().status()
            ),
            Err(err) => err,
        };
        assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_UPDATE_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_item_id.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn delete_software_item_missing_item_writes_denied_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let missing_item_id = Uuid::now_v7();

        let response = delete_software_item(
            State(AuditEmitterState(state.audit_emitter.clone())),
            tenant_db,
            CanDeleteSoftware::new(auth_user_with(Permission::DeleteSoftware)),
            None,
            Path(missing_item_id),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_DELETE_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_item.not_found")
        );
    }

    #[tokio::test]
    async fn approve_software_item_already_featured_writes_denied_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let item_id = Uuid::now_v7();
        insert_software_item_row_with_flags(&db, tenant_id, item_id, "Featured App", true).await;

        let response = approve_software_item(
            State(AuditEmitterState(state.audit_emitter.clone())),
            tenant_db,
            CanUpdateSoftware::new(auth_user_with(Permission::UpdateSoftware)),
            None,
            Path(item_id),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_APPROVE_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_item.already_featured")
        );
    }

    #[tokio::test]
    async fn assign_hosts_empty_payload_writes_validation_failed_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let item_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;

        let response = match assign_hosts(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateSoftware::new(auth_user_with(Permission::UpdateSoftware)),
            None,
            Path(item_id),
            Json(AssignHostsRequest {
                host_assignments: vec![],
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(err) => panic!(
                "empty payload should return bad-request response, got status {}",
                err.into_response().status()
            ),
        };
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_item.host_assignments_empty")
        );
    }

    #[tokio::test]
    async fn unassign_host_missing_assignment_writes_denied_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;

        let response = unassign_host(
            State(AuditEmitterState(state.audit_emitter.clone())),
            tenant_db,
            CanUpdateSoftware::new(auth_user_with(Permission::UpdateSoftware)),
            None,
            Path((item_id, host_id)),
            Query(DeleteHostAssignmentParams {
                ignore: Some(false),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_item.assignment_not_found")
        );
    }

    #[tokio::test]
    async fn update_host_assignment_missing_item_writes_denied_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();

        let err = match update_host_assignment(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateSoftware::new(auth_user_with(Permission::UpdateSoftware)),
            None,
            Path((item_id, host_id)),
            Json(UpdateHostAssignmentRequest {
                role: PluginRole::DetectVersion,
                ordinal: 0,
                plugin_config_id: None,
                plugin_config: None,
                plugin_type: None,
                package_identifier: Some("pkg".to_string()),
                config_override: Default::default(),
                execution_site: None,
            }),
        )
        .await
        {
            Ok(response) => panic!(
                "update host assignment should fail, got status {}",
                response.into_response().status()
            ),
            Err(err) => err,
        };
        assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);

        let row =
            tenant_audit_row_for_action(&db, SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION)
                .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
    }

    #[tokio::test]
    async fn delete_plugin_assignment_invalid_role_writes_validation_failed_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();

        let response = match delete_plugin_assignment(
            State(AuditEmitterState(state.audit_emitter.clone())),
            tenant_db,
            CanUpdateSoftware::new(auth_user_with(Permission::UpdateSoftware)),
            None,
            Path((item_id, host_id, "invalid_role".to_string(), 0)),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(err) => panic!(
                "invalid role should return bad-request response, got status {}",
                err.into_response().status()
            ),
        };
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let row =
            tenant_audit_row_for_action(&db, SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION)
                .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_item.invalid_role")
        );
    }

    #[tokio::test]
    async fn execute_merge_invalid_request_writes_validation_failed_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let only_id = Uuid::now_v7();

        let err = match execute_software_item_merge(
            State(AuditEmitterState(state.audit_emitter.clone())),
            tenant_db,
            CanUpdateSoftware::new(auth_user_with(Permission::UpdateSoftware)),
            CanDeleteSoftware::new(auth_user_with(Permission::DeleteSoftware)),
            None,
            Json(MergeSoftwareItemsExecuteRequest {
                candidate_ids: vec![only_id],
                survivor_id: only_id,
            }),
        )
        .await
        {
            Ok(response) => panic!(
                "invalid merge request should fail, got status {}",
                response.into_response().status()
            ),
            Err(err) => err,
        };
        assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_MERGE_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_item.invalid_merge_request")
        );
    }

    #[tokio::test]
    async fn batch_software_items_partial_result_writes_partial_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let existing_item_id = Uuid::now_v7();
        let missing_item_id = Uuid::now_v7();
        insert_software_item_row_with_flags(
            &db,
            tenant_id,
            existing_item_id,
            "Batch Partial App",
            false,
        )
        .await;

        let response = batch_software_items(
            State(Arc::clone(&state)),
            tenant_db,
            CanDeleteSoftware::new(auth_user_with(Permission::DeleteSoftware)),
            None,
            Validated(BatchActionRequest {
                action: "approve".to_string(),
                ids: vec![existing_item_id, missing_item_id],
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_BATCH_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Partial.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["requested_count"], serde_json::json!(2));
        assert_eq!(details["succeeded_count"], serde_json::json!(1));
        assert_eq!(details["failed_count"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn batch_software_items_unknown_action_writes_validation_failed_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
        let item_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;

        let response = batch_software_items(
            State(Arc::clone(&state)),
            tenant_db,
            CanDeleteSoftware::new(auth_user_with(Permission::DeleteSoftware)),
            None,
            Validated(BatchActionRequest {
                action: "invalid".to_string(),
                ids: vec![item_id],
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(&db, SOFTWARE_ITEM_BATCH_AUDIT_ACTION).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("software_item.batch_unknown_action")
        );
    }

    #[tokio::test]
    async fn trigger_update_writes_software_update_triggered_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

        let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
        let service = crate::test_harness::fixtures::insert_service(
            &db,
            tenant_id,
            service::ServiceStatus::Approved,
        )
        .await;
        crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

        let item_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;
        let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
        insert_execute_update_plugin(&db, host.id, item_id, host_software_item_id).await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::TriggerUpdates],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = trigger_update(
            State(Arc::clone(&state)),
            tenant_db,
            CanTriggerUpdates::new(auth_user),
            None,
            Path((item_id, host.id)),
            Json(TriggerUpdateRequest {
                to_version: "1.1.0".to_string(),
                release_info: None,
                interactive: false,
            }),
        )
        .await;

        let response = match response {
            Ok(response) => response,
            Err(err) => panic!(
                "trigger_update should succeed, got status {}",
                err.into_response().status()
            ),
        };
        assert_eq!(response.into_response().status(), StatusCode::OK);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(row.target_id.as_deref(), Some(item_id.to_string().as_str()));
        let details = row.details_json.expect("details");
        assert_eq!(details["host_id"], serde_json::json!(host.id));
        assert_eq!(details["to_version"], serde_json::json!("1.1.0"));
        assert_eq!(details["interactive"], serde_json::json!(false));
        assert_eq!(details["dispatch_status"], serde_json::json!("pending"));

        let update_row = update_history::Entity::find()
            .filter(update_history::Column::SoftwareItemId.eq(item_id))
            .filter(update_history::Column::HostId.eq(host.id))
            .one(&db)
            .await
            .expect("query update history")
            .expect("update history row");
        assert_eq!(update_row.actor_type, ActorType::User.as_str());
    }

    #[tokio::test]
    async fn trigger_update_host_not_assigned_writes_validation_failed_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

        let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
        let service = crate::test_harness::fixtures::insert_service(
            &db,
            tenant_id,
            service::ServiceStatus::Approved,
        )
        .await;
        crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

        let item_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::TriggerUpdates],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = trigger_update(
            State(Arc::clone(&state)),
            tenant_db,
            CanTriggerUpdates::new(auth_user),
            None,
            Path((item_id, host.id)),
            Json(TriggerUpdateRequest {
                to_version: "1.1.0".to_string(),
                release_info: None,
                interactive: false,
            }),
        )
        .await;

        let error = match response {
            Ok(response) => panic!(
                "trigger_update should fail with host-not-assigned, got status {}",
                response.into_response().status()
            ),
            Err(err) => err,
        };
        assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(row.target_id.as_deref(), Some(item_id.to_string().as_str()));
        let details = row.details_json.expect("details");
        assert_eq!(details["host_id"], serde_json::json!(host.id));
        assert_eq!(details["to_version"], serde_json::json!("1.1.0"));
        assert_eq!(details["interactive"], serde_json::json!(false));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("trigger_update.host_not_assigned")
        );
    }

    #[tokio::test]
    async fn trigger_update_missing_item_writes_denied_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

        let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
        let service = crate::test_harness::fixtures::insert_service(
            &db,
            tenant_id,
            service::ServiceStatus::Approved,
        )
        .await;
        crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

        let missing_item_id = Uuid::now_v7();

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::TriggerUpdates],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = trigger_update(
            State(Arc::clone(&state)),
            tenant_db,
            CanTriggerUpdates::new(auth_user),
            None,
            Path((missing_item_id, host.id)),
            Json(TriggerUpdateRequest {
                to_version: "1.1.0".to_string(),
                release_info: None,
                interactive: true,
            }),
        )
        .await;

        let error = match response {
            Ok(response) => panic!(
                "trigger_update should fail with software-item-not-found, got status {}",
                response.into_response().status()
            ),
            Err(err) => err,
        };
        assert_eq!(error.into_response().status(), StatusCode::NOT_FOUND);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(missing_item_id.to_string().as_str())
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["host_id"], serde_json::json!(host.id));
        assert_eq!(details["to_version"], serde_json::json!("1.1.0"));
        assert_eq!(details["interactive"], serde_json::json!(true));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("trigger_update.software_item_not_found")
        );
    }

    #[tokio::test]
    async fn trigger_update_with_api_token_actor_writes_api_token_actor_id() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

        let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
        let service = crate::test_harness::fixtures::insert_service(
            &db,
            tenant_id,
            service::ServiceStatus::Approved,
        )
        .await;
        crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

        let item_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;
        let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
        insert_execute_update_plugin(&db, host.id, item_id, host_software_item_id).await;

        let token_id = Uuid::now_v7();
        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::ApiToken,
            permissions: vec![Permission::TriggerUpdates],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = trigger_update(
            State(Arc::clone(&state)),
            tenant_db,
            CanTriggerUpdates::new(auth_user),
            Some(Extension(AuthenticatedApiTokenId(token_id))),
            Path((item_id, host.id)),
            Json(TriggerUpdateRequest {
                to_version: "1.1.0".to_string(),
                release_info: None,
                interactive: false,
            }),
        )
        .await;

        let response = match response {
            Ok(response) => response,
            Err(err) => panic!(
                "trigger_update should succeed, got status {}",
                err.into_response().status()
            ),
        };
        assert_eq!(response.into_response().status(), StatusCode::OK);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert_eq!(row.actor_id, Some(token_id));

        let update_row = update_history::Entity::find()
            .filter(update_history::Column::SoftwareItemId.eq(item_id))
            .filter(update_history::Column::HostId.eq(host.id))
            .one(&db)
            .await
            .expect("query update history")
            .expect("update history row");
        assert_eq!(update_row.actor_type, ActorType::ApiToken.as_str());
        assert_eq!(update_row.actor_id, token_id.to_string());
    }

    #[tokio::test]
    async fn check_versions_writes_software_version_check_triggered_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

        let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
        let service = crate::test_harness::fixtures::insert_service(
            &db,
            tenant_id,
            service::ServiceStatus::Approved,
        )
        .await;
        crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

        let item_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;
        let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
        insert_detect_version_plugin(&db, host.id, item_id, host_software_item_id, "agent").await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::TriggerChecks],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = check_versions(
            State(Arc::clone(&state)),
            tenant_db,
            CanTriggerChecks::new(auth_user),
            None,
            Path(item_id),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(row.target_id.as_deref(), Some(item_id.to_string().as_str()));
        let details = row.details_json.expect("details");
        assert_eq!(details["dispatch_scope"], serde_json::json!("all_hosts"));
        assert_eq!(details["agents_notified"], serde_json::json!(1));
        assert_eq!(details["controller_checks_run"], serde_json::json!(0));
    }

    #[tokio::test]
    async fn check_versions_host_writes_software_version_check_triggered_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

        let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
        let service = crate::test_harness::fixtures::insert_service(
            &db,
            tenant_id,
            service::ServiceStatus::Approved,
        )
        .await;
        crate::test_harness::fixtures::link_service_host(&db, service.id, host.id).await;

        let item_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;
        let host_software_item_id = insert_host_assignment(&db, host.id, item_id).await;
        insert_detect_version_plugin(&db, host.id, item_id, host_software_item_id, "agent").await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::TriggerChecks],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = check_versions_host(
            State(Arc::clone(&state)),
            tenant_db,
            CanTriggerChecks::new(auth_user),
            None,
            Path((item_id, host.id)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(row.target_id.as_deref(), Some(item_id.to_string().as_str()));
        let details = row.details_json.expect("details");
        assert_eq!(details["dispatch_scope"], serde_json::json!("single_host"));
        assert_eq!(details["host_id"], serde_json::json!(host.id));
        assert_eq!(details["agents_notified"], serde_json::json!(1));
        assert_eq!(details["controller_checks_run"], serde_json::json!(0));
    }

    #[tokio::test]
    async fn check_versions_host_missing_assignment_writes_validation_failed_audit_event() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;

        let host = crate::test_harness::fixtures::insert_host(&db, tenant_id).await;
        let item_id = Uuid::now_v7();
        insert_software_item_row(&db, tenant_id, item_id).await;

        let auth_user = AuthenticatedUser {
            user_id: Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::TriggerChecks],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = check_versions_host(
            State(Arc::clone(&state)),
            tenant_db,
            CanTriggerChecks::new(auth_user),
            None,
            Path((item_id, host.id)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["dispatch_scope"], serde_json::json!("single_host"));
        assert_eq!(details["host_id"], serde_json::json!(host.id));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("version_check.host_not_assigned")
        );
    }
}
