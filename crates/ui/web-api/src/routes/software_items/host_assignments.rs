//! Host and plugin-assignment handlers for `/api/v1/software-items/{id}/hosts`.
//!
//! Covers: assign_hosts, unassign_host, update_host_assignment, delete_plugin_assignment.

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::action::CanUpdateSoftware;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::software_items as item_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_web_api_queries::queries::software_items::SoftwareItemView;
use uptrakit_web_api_types::PluginRole;
use uuid::Uuid;

use super::audit::{
    SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION, SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
    SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION, SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION,
};
use super::{AssignHostsRequest, SoftwareItemDetailResponse, UpdateHostAssignmentRequest};

/// Assign a software item to additional hosts.
///
/// Each host in `host_assignments` carries its own `plugin_config_id`,
/// `package_identifier`, and optional `config_override`.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    request_body = AssignHostsRequest,
    responses(
        (status = 200, description = "Hosts assigned", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found")
    ),
    tag = "Software Items",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn assign_hosts(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<AssignHostsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    if req.host_assignments.is_empty() {
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::ValidationFailed)
        .details(serde_json::json!({ "reason_code": "software_item.host_assignments_empty" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(
            StatusCode::BAD_REQUEST,
            "host_assignments must not be empty",
        );
    }

    // Pre-read the item to build the AuditView (before snapshot).
    let item_model = match item_queries::find_active_item(tenant_db.db(), tenant_id, item_id).await
    {
        Some(m) => m,
        None => {
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION,
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
    };

    let item_view = SoftwareItemView::from(&item_model);
    let assignment_count = req.host_assignments.len();

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
            tracing::error!(error = %e, "Failed to begin transaction for assign_hosts");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(err) = item_queries::assign_hosts_in_tx(
        state.plugin.plugin_ops.as_ref(),
        &tx,
        tenant_id,
        item_id,
        &req,
    )
    .await
    {
        let (outcome, reason_code) = err.current_context().audit_classification();
        drop(tx);
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION,
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
            item_queries::SoftwareItemQueryError::NotFound => {
                error_response(StatusCode::NOT_FOUND, "Software item not found")
            }
            item_queries::SoftwareItemQueryError::HostNotFound(_) => {
                error_response(StatusCode::NOT_FOUND, "Host not found")
            }
            item_queries::SoftwareItemQueryError::PluginConfigNotFound => {
                error_response(StatusCode::NOT_FOUND, "Plugin config not found")
            }
            item_queries::SoftwareItemQueryError::InvalidPackageIdentifier(msg) => {
                error_response(StatusCode::BAD_REQUEST, msg.as_str())
            }
            item_queries::SoftwareItemQueryError::InvalidConfigOverride(msg) => {
                error_response(StatusCode::BAD_REQUEST, msg.as_str())
            }
            item_queries::SoftwareItemQueryError::DuplicateHostAssignment => {
                error_response(StatusCode::CONFLICT, "Duplicate host assignment")
            }
            _ => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
        };
    }

    let hook = state.audit_emitter.commit_hook();
    let audit_entry =
        match AuditEntry::<Stateful>::software_item_assign_hosts(&item_view, &item_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({ "assigned_hosts_count": assignment_count }))
            .build()
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::error!(error = %e, "Failed to build audit entry for assign_hosts");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for assign_hosts");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit assign_hosts");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    match item_queries::get_software_item(&tenant_db, item_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) | Err(_) => {
            tracing::warn!(%item_id, "software item not found after successful assign_hosts commit");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Item not found after update",
            )
        }
    }
}

#[derive(serde::Deserialize, Default, utoipa::IntoParams)]
pub struct DeleteHostAssignmentParams {
    /// If true, permanently suppress items with this name from future autodiscovery runs.
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
        DeleteHostAssignmentParams
    ),
    responses(
        (status = 204, description = "Host unassigned"),
        (status = 404, description = "Software item or host assignment not found")
    ),
    tag = "Software Items",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn unassign_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<DeleteHostAssignmentParams>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();
    let ignore_requested = params.ignore.unwrap_or(false);

    // Pre-read the item to build the AuditView and optionally get the name for
    // the autodiscovery ignore rule.
    let item_model = match item_queries::find_active_item(tenant_db.db(), tenant_id, item_id).await
    {
        Some(m) => m,
        None => {
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "host_id": host_id,
                "reason_code": "software_item.not_found",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Software item not found");
        }
    };

    let item_view = SoftwareItemView::from(&item_model);
    let ignore_name = if ignore_requested {
        Some(item_model.name.clone())
    } else {
        None
    };

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
            tracing::error!(error = %e, "Failed to begin transaction for unassign_host");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let found = match item_queries::unassign_host_in_tx(&tx, item_id, host_id).await {
        Ok(found) => found,
        Err(e) => {
            tracing::error!(error = %e, "Failed to unassign host from software item");
            drop(tx);
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({
                "host_id": host_id,
                "reason_code": "software_item.database_error",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !found {
        drop(tx);
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "host_id": host_id,
            "reason_code": "software_item.assignment_not_found",
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(
            StatusCode::NOT_FOUND,
            "Software item or host assignment not found",
        );
    }

    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::software_item_unassign_host(
        &item_view,
        &AbsentView(&item_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "host_id": host_id,
        "ignore_requested": ignore_requested,
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for unassign_host");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for unassign_host");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit unassign_host");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // Optionally create an autodiscovery ignore rule after commit.
    let mut ignore_rule_created = false;
    if let Some(name) = ignore_name {
        match autodiscovery_queries::create_or_ignore_ignore_rule(
            tenant_db.db(),
            tenant_id,
            &name,
            None,
        )
        .await
        {
            Ok(_) => {
                ignore_rule_created = true;
            }
            Err(e) => {
                tracing::warn!("Failed to create autodiscovery ignore rule: {e}");
            }
        }
    }

    if ignore_requested && !ignore_rule_created {
        tracing::debug!(%item_id, "ignore requested but rule not created (may already exist)");
    }

    StatusCode::NO_CONTENT.into_response()
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
    responses(
        (status = 200, description = "Host assignment updated", body = SoftwareItemDetailResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item or host assignment not found"),
        (status = 409, description = "Duplicate host assignment")
    ),
    tag = "Software Items",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_host_assignment(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateHostAssignmentRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();
    let role_str = req.role.as_str().to_string();
    let ordinal = req.ordinal;

    // Pre-read the item to build the AuditView.
    let item_model = match item_queries::find_active_item(tenant_db.db(), tenant_id, item_id).await
    {
        Some(m) => m,
        None => {
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "host_id": host_id,
                "role": role_str,
                "ordinal": ordinal,
                "reason_code": "software_item.not_found",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Software item not found");
        }
    };

    let item_view = SoftwareItemView::from(&item_model);

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
            tracing::error!(error = %e, "Failed to begin transaction for update_host_assignment");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(err) = item_queries::update_host_assignment_in_tx(
        state.plugin.plugin_ops.as_ref(),
        &tx,
        tenant_id,
        item_id,
        host_id,
        req,
    )
    .await
    {
        let (outcome, reason_code) = err.current_context().audit_classification();
        drop(tx);
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(serde_json::json!({
            "host_id": host_id,
            "role": role_str,
            "ordinal": ordinal,
            "reason_code": reason_code,
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        let ctx = err.current_context();
        return match ctx {
            item_queries::SoftwareItemQueryError::NotFound => error_response(
                StatusCode::NOT_FOUND,
                "Software item or host assignment not found",
            ),
            item_queries::SoftwareItemQueryError::HostNotFound(_) => {
                error_response(StatusCode::NOT_FOUND, "Host not found")
            }
            item_queries::SoftwareItemQueryError::PluginConfigNotFound => {
                error_response(StatusCode::NOT_FOUND, "Plugin config not found")
            }
            item_queries::SoftwareItemQueryError::InvalidPackageIdentifier(msg) => {
                error_response(StatusCode::BAD_REQUEST, msg.as_str())
            }
            item_queries::SoftwareItemQueryError::InvalidConfigOverride(msg) => {
                error_response(StatusCode::BAD_REQUEST, msg.as_str())
            }
            item_queries::SoftwareItemQueryError::DuplicateHostAssignment => {
                error_response(StatusCode::CONFLICT, "Duplicate host assignment")
            }
            _ => error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
        };
    }

    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::software_item_update_host_assignment(
        &item_view, &item_view,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "host_id": host_id,
        "role": role_str,
        "ordinal": ordinal,
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for update_host_assignment");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for update_host_assignment");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit update_host_assignment");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    match item_queries::get_software_item(&tenant_db, item_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) | Err(_) => {
            tracing::warn!(%item_id, "software item not found after successful update_host_assignment commit");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Item not found after update",
            )
        }
    }
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
    responses(
        (status = 200, description = "Plugin assignment removed", body = SoftwareItemDetailResponse),
        (status = 404, description = "Software item, host, or plugin assignment not found"),
    ),
    tag = "Software Items",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_plugin_assignment(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id, role, ordinal)): Path<(Uuid, Uuid, String, i32)>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();
    let requested_role = role.clone();

    let role = match role.parse::<PluginRole>() {
        Ok(r) => r,
        Err(_) => {
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({
                "host_id": host_id,
                "role": requested_role,
                "ordinal": ordinal,
                "reason_code": "software_item.invalid_role",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::BAD_REQUEST, "invalid role");
        }
    };

    let role_str = role.as_str().to_string();

    // Pre-read the item to build the AuditView.
    let item_model = match item_queries::find_active_item(tenant_db.db(), tenant_id, item_id).await
    {
        Some(m) => m,
        None => {
            if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "host_id": host_id,
                "role": role_str,
                "ordinal": ordinal,
                "reason_code": "software_item.not_found",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Software item not found");
        }
    };

    let item_view = SoftwareItemView::from(&item_model);

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
            tracing::error!(error = %e, "Failed to begin transaction for delete_plugin_assignment");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let found =
        match item_queries::delete_plugin_assignment_in_tx(&tx, item_id, host_id, &role, ordinal)
            .await
        {
            Ok(found) => found,
            Err(err) => {
                let (outcome, reason_code) = err.current_context().audit_classification();
                drop(tx);
                if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
                    SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .outcome(outcome)
                .details(serde_json::json!({
                    "host_id": host_id,
                    "role": role_str,
                    "ordinal": ordinal,
                    "reason_code": reason_code,
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if !found {
        drop(tx);
        if let Ok(entry) = AuditEntry::<uptrakit_audit_log::Event>::builder_event(
            SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "host_id": host_id,
            "role": role_str,
            "ordinal": ordinal,
            "reason_code": "software_item.plugin_assignment_not_found",
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(
            StatusCode::NOT_FOUND,
            "Software item, host, or plugin assignment not found",
        );
    }

    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::software_item_delete_plugin_assignment(
        &item_view,
        &AbsentView(&item_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "host_id": host_id,
        "role": role_str,
        "ordinal": ordinal,
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for delete_plugin_assignment");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for delete_plugin_assignment");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit delete_plugin_assignment");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    match item_queries::get_software_item(&tenant_db, item_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) | Err(_) => {
            tracing::warn!(%item_id, "software item not found after successful delete_plugin_assignment commit");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Item not found after update",
            )
        }
    }
}
