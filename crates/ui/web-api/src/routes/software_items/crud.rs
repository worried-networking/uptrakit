//! CRUD handlers for `/api/v1/software-items`.
//!
//! Covers: create, list, get, update, delete, approve.
//! The `fire_software_item_lifecycle` helper is private to this module —
//! it is only called by `create_software_item`.

use crate::AppState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{
    CanCreateSoftware, CanDeleteSoftware, CanUpdateSoftware, CanViewSoftware,
};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::plugin_type_settings as pts_queries;
use crate::queries::software_items as item_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, SqliteTransactionMode, TransactionOptions,
    TransactionTrait,
};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_shared_db::entity::software_item;
use uptrakit_web_api_queries::queries::software_items::SoftwareItemView;
use uptrakit_web_api_types::events::AdminEvent;
use uuid::Uuid;

use super::audit::{
    SOFTWARE_ITEM_APPROVE_AUDIT_ACTION, SOFTWARE_ITEM_CREATE_AUDIT_ACTION,
    SOFTWARE_ITEM_DELETE_AUDIT_ACTION, SOFTWARE_ITEM_UPDATE_AUDIT_ACTION,
};
use super::{
    CreateSoftwareItemRequest, ListSoftwareItemsParams, PaginatedResponse,
    SoftwareItemDetailResponse, SoftwareItemResponse, UpdateSoftwareItemRequest,
};

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
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction for software item create");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let inserted = match item_queries::create_software_item_in_tx(&tx, tenant_id, &req).await {
        Ok(m) => m,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            drop(tx);
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_CREATE_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(outcome)
            .details(serde_json::json!({ "reason_code": reason_code }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            let ctx = err.current_context();
            return match ctx {
                item_queries::SoftwareItemQueryError::DuplicateItem => error_response(
                    StatusCode::CONFLICT,
                    "A software item with this name already exists",
                ),
                _ => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
            };
        }
    };

    let after_view = SoftwareItemView::from(&inserted);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry =
        match AuditEntry::<Stateful>::software_item_create(&AbsentView(&after_view), &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({
                "featured": inserted.featured,
                "has_icon_url": inserted.icon_url.is_some(),
            }))
            .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!(error = %e, "Failed to build audit entry for software item create");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for software item create");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit software item create");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // Build the API response from the inserted model.
    let mut resp = uptrakit_web_api_types::software_items::SoftwareItemResponse {
        id: inserted.id,
        name: inserted.name.clone(),
        plugins: vec![],
        featured: inserted.featured,
        last_checked_at: inserted.last_checked_at,
        host_count: 0,
        installed_version: None,
        installed_display_version: None,
        latest_version: None,
        latest_release_metadata: None,
        update_available: false,
        created_at: inserted.created_at,
        updated_at: inserted.updated_at,
        icon_url: inserted.icon_url.clone(),
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
        .send(tenant_id, AdminEvent::SoftwareItemCreated { id: resp.id })
        .await;

    (StatusCode::CREATED, Json(resp)).into_response()
}

/// List all active software items (with host count).
#[utoipa::path(
    get,
    path = "/api/v1/software-items",
    params(ListSoftwareItemsParams),
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
    if params
        .query
        .as_deref()
        .is_some_and(|q| q.chars().count() > 200)
    {
        return error_response(StatusCode::BAD_REQUEST, "query too long");
    }
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
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<UpdateSoftwareItemRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let name_changed = req.name.is_some();
    let featured_changed = req.featured.is_some();
    let icon_url_changed = !req.icon_url.is_keep();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction for software item update");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let (before_model, after_model) =
        match item_queries::update_software_item_in_tx(&tx, tenant_id, item_id, &req).await {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                drop(tx);
                if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                    SOFTWARE_ITEM_UPDATE_AUDIT_ACTION,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target("software_item", item_id.to_string(), None)
                .outcome(AuditOutcome::Denied)
                .details(serde_json::json!({ "reason_code": "software_item.not_found" }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::NOT_FOUND, "Software item not found");
            }
            Err(err) => {
                let (outcome, reason_code) = err.current_context().audit_classification();
                drop(tx);
                if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                    SOFTWARE_ITEM_UPDATE_AUDIT_ACTION,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target("software_item", item_id.to_string(), None)
                .outcome(outcome)
                .details(serde_json::json!({ "reason_code": reason_code }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                let ctx = err.current_context();
                return match ctx {
                    item_queries::SoftwareItemQueryError::EmptyName => {
                        error_response(StatusCode::BAD_REQUEST, "name must not be empty")
                    }
                    item_queries::SoftwareItemQueryError::DuplicateItem => error_response(
                        StatusCode::CONFLICT,
                        "A software item with this name already exists",
                    ),
                    _ => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
                };
            }
        };

    let before_view = SoftwareItemView::from(&before_model);
    let after_view = SoftwareItemView::from(&after_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::software_item_update(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "changed_fields": {
                "name": name_changed,
                "featured": featured_changed,
                "icon_url": icon_url_changed,
            }
        }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for software item update");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for software item update");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit software item update");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // Broadcast
    state
        .notification
        .event_broadcaster
        .send(tenant_id, AdminEvent::SoftwareItemUpdated { id: item_id })
        .await;

    // Load enriched list-style response after commit (read-only, same as the
    // original update_software_item path which queries post-update).
    match item_queries::get_software_item(&tenant_db, item_id).await {
        Ok(Some(detail)) => {
            // Convert detail to the list response shape expected by callers.
            let resp = uptrakit_web_api_types::software_items::SoftwareItemResponse {
                id: detail.id,
                name: detail.name,
                plugins: detail.plugins,
                featured: detail.featured,
                last_checked_at: detail.last_checked_at,
                host_count: detail.host_count,
                installed_version: None,
                installed_display_version: None,
                latest_version: detail.latest_version,
                latest_release_metadata: None,
                update_available: detail.update_available,
                created_at: detail.created_at,
                updated_at: detail.updated_at,
                icon_url: detail.icon_url,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Ok(None) | Err(_) => {
            // After a successful commit the item must exist; this should not happen.
            tracing::warn!(%item_id, "software item not found after successful update commit");
            let resp = uptrakit_web_api_types::software_items::SoftwareItemResponse {
                id: after_model.id,
                name: after_model.name,
                plugins: vec![],
                featured: after_model.featured,
                last_checked_at: after_model.last_checked_at,
                host_count: 0,
                installed_version: None,
                installed_display_version: None,
                latest_version: None,
                latest_release_metadata: None,
                update_available: false,
                created_at: after_model.created_at,
                updated_at: after_model.updated_at,
                icon_url: after_model.icon_url,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanDeleteSoftware(user): CanDeleteSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction for software item delete");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model = match item_queries::delete_software_item_in_tx(&tx, tenant_id, item_id).await
    {
        Ok(Some(m)) => m,
        Ok(None) => {
            drop(tx);
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_DELETE_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({ "reason_code": "software_item.not_found" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Software item not found");
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to delete software item");
            drop(tx);
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_DELETE_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "software_item.database_error" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_view = SoftwareItemView::from(&before_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry =
        match AuditEntry::<Stateful>::software_item_delete(&before_view, &AbsentView(&before_view))
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({}))
            .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!(error = %e, "Failed to build audit entry for software item delete");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for software item delete");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit software item delete");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
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
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    // Read the item before the transaction to check if it is already featured.
    // This is a read-only pre-check; the actual write happens in the transaction.
    let existing = match software_item::Entity::find_by_id(item_id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(i)) => i,
        Ok(None) => {
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({ "reason_code": "software_item.not_found" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Software item not found");
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to fetch software item for approve");
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "software_item.database_error" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if existing.featured {
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({ "reason_code": "software_item.already_featured" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::CONFLICT, "Software item is already featured");
    }

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction for software item approve");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let (before_model, after_model) =
        match item_queries::approve_software_item_in_tx(&tx, tenant_id, item_id).await {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                drop(tx);
                if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                    SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Denied)
                .details(serde_json::json!({ "reason_code": "software_item.not_found" }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::NOT_FOUND, "Software item not found");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to approve software item");
                drop(tx);
                if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                    SOFTWARE_ITEM_APPROVE_AUDIT_ACTION,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({ "reason_code": "software_item.database_error" }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let before_view = SoftwareItemView::from(&before_model);
    let after_view = SoftwareItemView::from(&after_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::software_item_approve(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({ "featured": true }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for software item approve");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for software item approve");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit software item approve");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    match item_queries::get_software_item(&tenant_db, after_model.id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) | Err(_) => {
            tracing::warn!(%item_id, "software item not found after successful approve commit");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Item not found after update",
            )
        }
    }
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
        tenant_db.tenant_id(),
        resp.name.clone(),
        resp.featured,
        resp.icon_url.clone(),
    );

    let lifecycle_ctx = match pts_queries::preload_lifecycle_type_settings(
        tenant_db.db(),
        tenant_db.tenant_id(),
        state.plugin.plugin_ops.as_ref(),
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!(
                error = %e,
                tenant_id = %tenant_db.tenant_id(),
                "failed to preload lifecycle type settings; using defaults"
            );
            uptrakit_plugin_infrastructure_registry::SoftwareItemLifecycleContext::default()
        }
    };

    state
        .plugin
        .plugin_ops
        .on_software_item_created(&event, &lifecycle_ctx)
        .await
}
