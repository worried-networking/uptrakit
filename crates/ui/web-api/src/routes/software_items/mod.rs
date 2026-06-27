//! HTTP route handlers for `/api/v1/software-items`.
//!
//! Controller-side fetch orchestration lives in [`controller_fetch`].
//! Version-check context loading and agent dispatch live in [`version_check_dispatch`].

mod audit;
mod controller_fetch;
mod crud;
mod version_check_dispatch;

pub use crud::{
    approve_software_item, create_software_item, delete_software_item, get_software_item,
    list_software_items, update_software_item,
};
// Re-export utoipa `__path_*` types so `routes!(crate::routes::software_items::<handler>)`
// in router.rs resolves them at the facade's public path.
pub use crud::{
    __path_approve_software_item, __path_create_software_item, __path_delete_software_item,
    __path_get_software_item, __path_list_software_items, __path_update_software_item,
};

use crate::AppState;
use crate::actions::software_items as item_actions;
use crate::api_error::ApiError;
use crate::app_state::AuditEmitterState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{
    CanDeleteSoftware, CanTriggerChecks, CanTriggerUpdates, CanUpdateSoftware,
};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::autodiscovery as autodiscovery_queries;
use crate::queries::plugin_configs::find_raw_active_config;
use crate::queries::software_items as item_queries;
use crate::queries::update_types::ActorType;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, RelationTrait as _, SqliteTransactionMode,
    TransactionOptions, TransactionTrait,
};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_shared_db::entity::{
    host, host_software_item_plugin, prelude::*, service, service_host,
};
use uptrakit_shared_types::PluginTypeId;
use uptrakit_web_api_queries::queries::software_items::SoftwareItemView;
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

use audit::{
    AuditContext, SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION, SOFTWARE_ITEM_BATCH_AUDIT_ACTION,
    SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION, SOFTWARE_ITEM_MERGE_AUDIT_ACTION,
    SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION, SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION,
    emit_software_item_mutation_audit, emit_software_version_check_audit,
};
use controller_fetch::{ControllerFetchJob, is_controller_fetch_site, run_controller_fetch_jobs};
use version_check_dispatch::{
    collect_and_run_controller_fetches, dispatch_agent_version_checks, load_version_check_context,
};

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

// --- Endpoints ---

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
        tenant_id: tenant_db.tenant_id(),
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
    // Convert the API release_info type to the wire type then serialise for
    // UpdateDispatchParams, which carries an opaque JSON value.
    let release_info = req.release_info.map(|ri| {
        let wire = uptrakit_wire::ReleaseInfo {
            tag: ri.tag,
            release_url: ri.release_url,
            assets: ri
                .assets
                .into_iter()
                .map(|a| uptrakit_wire::ReleaseAsset {
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
        };
        serde_json::to_value(wire).unwrap_or_else(|e| {
            tracing::warn!("failed to serialize release info: {e}");
            serde_json::Value::Null
        })
    });

    let result = state
        .update_dispatcher
        .dispatch(uptrakit_controller_core::update::UpdateDispatchParams::new(
            tenant_db.tenant_id(),
            host_id,
            item_id,
            to_version.clone(),
            uptrakit_controller_core::update::ActorInfo::new(update_actor_type, update_actor_id),
            release_info,
            interactive,
        ))
        .await?;

    let status = match result.outcome {
        uptrakit_controller_core::update::DispatchOutcome::Sent => TriggerUpdateStatus::Pending,
        uptrakit_controller_core::update::DispatchOutcome::Queued => TriggerUpdateStatus::Queued,
        uptrakit_controller_core::update::DispatchOutcome::Failed => TriggerUpdateStatus::Failed,
        _ => {
            tracing::warn!("unhandled DispatchOutcome in HTTP trigger_update response mapping");
            TriggerUpdateStatus::Failed
        }
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
    CanTriggerChecks(user): CanTriggerChecks,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };

    // Verify software item exists and is active
    let item = match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id(), item_id)
        .await
    {
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
    let item = match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id(), item_id)
        .await
    {
        Some(i) => i,
        None => return Err(CheckVersionsHostPreconditionError::SoftwareItemNotFound),
    };

    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
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
        .filter(service::Column::TenantId.eq(tenant_db.tenant_id()))
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
        Option<uptrakit_wire::PluginAssignment>,
        Option<uptrakit_wire::PluginAssignment>,
    ),
    Response,
> {
    let mut detect_version: Option<uptrakit_wire::PluginAssignment> = None;
    let mut fetch_releases: Option<uptrakit_wire::PluginAssignment> = None;
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
        let pa = uptrakit_wire::PluginAssignment {
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
        tenant_id: tenant_db.tenant_id(),
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
        tenant_db.tenant_id(),
        Some(state.global_providers()),
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
    let assignment = uptrakit_wire::VersionCheckAssignment {
        software_item_id: item_id,
        name: item.name.clone(),
        detect_version,
        fetch_releases,
        host_software_item_id: Some(link.id),
    };

    let msg =
        uptrakit_wire::ControllerMessage::CheckVersions(uptrakit_wire::CheckVersionsPayload {
            host_machine_id: host_record.machine_id.clone(),
            assignments: vec![assignment],
        });
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
        tenant_id: tenant_db.tenant_id(),
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

#[cfg(all(test, feature = "db-sqlite"))]
mod audit_tests;
#[cfg(all(test, feature = "db-sqlite"))]
mod tests;
