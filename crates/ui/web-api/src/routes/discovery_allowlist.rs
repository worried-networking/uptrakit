//! HTTP route handlers for the discovery plugin allowlist.
//!
//! Endpoints:
//! - `GET  /api/v1/discovery-allowlist`                           — list tenant-wide entries
//! - `POST /api/v1/discovery-allowlist`                           — add tenant-wide entry
//! - `DELETE /api/v1/discovery-allowlist/{id}`                    — remove tenant-wide entry
//! - `GET  /api/v1/hosts/{id}/discovery-allowlist`                — list host-specific entries
//! - `POST /api/v1/hosts/{id}/discovery-allowlist`                — add host-specific entry
//! - `DELETE /api/v1/hosts/{id}/discovery-allowlist/{entry_id}`   — remove host-specific entry

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, SqliteTransactionMode, TransactionOptions,
    TransactionTrait,
};
use uuid::Uuid;

use crate::AppState;
use crate::api_error::ApiError;
use crate::error_response::error_response;
use crate::extract::Unvalidated;
use crate::middleware::action::{CanReadSoftware, CanUpdateSoftware};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::discovery_allowlist as allowlist_queries;
use crate::tenant_db::TenantDb;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_shared_db::entity::{host, prelude::*};
pub use uptrakit_web_api_types::discovery_allowlist::{
    CreateDiscoveryAllowlistEntryRequest, HostDiscoveryAllowlistEntry,
    TenantDiscoveryAllowlistEntry,
};

// ── Tenant-wide endpoints ─────────────────────────────────────────────────────

/// List all tenant-wide discovery allowlist entries.
///
/// An empty list means no restrictions are configured — all discovery plugin
/// types will run (the "unconfigured = all allowed" default).
#[utoipa::path(
    get,
    path = "/api/v1/discovery-allowlist",
    responses(
        (status = 200, description = "Tenant-wide discovery allowlist entries", body = Vec<TenantDiscoveryAllowlistEntry>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["software:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_tenant_discovery_allowlist(
    tenant_db: TenantDb,
    CanReadSoftware(_user): CanReadSoftware,
) -> Response {
    match allowlist_queries::list_tenant_allowlist(tenant_db.db(), tenant_db.tenant_id()).await {
        Ok(entries) => (StatusCode::OK, Json(entries)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list tenant discovery allowlist");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Add a plugin type to the tenant-wide discovery allowlist.
///
/// Once any entry is added, only the listed plugin types will run discovery
/// tenant-wide (explicit allowlist semantics). Adding a duplicate entry returns
/// the existing entry with 201.
#[utoipa::path(
    post,
    path = "/api/v1/discovery-allowlist",
    request_body = CreateDiscoveryAllowlistEntryRequest,
    responses(
        (status = 201, description = "Entry created (or existing entry returned)", body = TenantDiscoveryAllowlistEntry),
        (status = 400, description = "Invalid or non-discovery plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn add_tenant_discovery_allowlist_entry(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    body: Unvalidated<CreateDiscoveryAllowlistEntryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({
                "scope": "tenant",
                "reason_code": "invalid_request",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
        }
    };
    let plugin_type_str = req.plugin_type.to_string();

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
            tracing::error!(error = %e, "Failed to begin transaction for tenant allowlist create");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let (was_created, entry) = match allowlist_queries::add_tenant_allowlist_entry_in_tx(
        state.plugin.plugin_ops.as_ref(),
        &tx,
        tenant_id,
        req.plugin_type,
    )
    .await
    {
        Ok(result) => result,
        Err(report) => {
            drop(tx);
            let (outcome, reason_code) = report
                .current_context()
                .tenant_create_audit_classification();
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(outcome)
            .details(serde_json::json!({
                "scope": "tenant",
                "plugin_type": plugin_type_str,
                "reason_code": reason_code,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return Err(report.into());
        }
    };

    if !was_created {
        // Entry already existed — no state change.
        drop(tx);
        if let Ok(audit_entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target(
            "discovery_allowlist_entry",
            entry.id.to_string(),
            Some(entry.plugin_type.clone()),
        )
        .outcome(AuditOutcome::Partial)
        .details(serde_json::json!({
            "scope": "tenant",
            "plugin_type": entry.plugin_type,
            "host_id": serde_json::Value::Null,
            "was_created": false,
        }))
        .build()
        {
            state.audit_emitter.emit_event(audit_entry);
        }
        return Ok((StatusCode::CREATED, Json(entry)).into_response());
    }

    // Newly inserted — emit_stateful.
    let view = allowlist_queries::TenantAllowlistView::from(&entry);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::discovery_allowlist_create(
        &AbsentView(&view),
        &view,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "scope": "tenant",
        "plugin_type": entry.plugin_type,
        "host_id": serde_json::Value::Null,
        "was_created": true,
    }))
    .build()
    {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for tenant allowlist create");
            drop(tx);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for tenant allowlist create");
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit tenant allowlist create");
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;

    Ok((StatusCode::CREATED, Json(entry)).into_response())
}

/// Remove a tenant-wide discovery allowlist entry.
///
/// Removing all entries restores the "unconfigured = all allowed" default.
#[utoipa::path(
    delete,
    path = "/api/v1/discovery-allowlist/{id}",
    params(("id" = Uuid, Path, description = "Allowlist entry UUID")),
    responses(
        (status = 204, description = "Entry removed"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Entry not found")
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn remove_tenant_discovery_allowlist_entry(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(entry_id): Path<Uuid>,
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
            tracing::error!(error = %e, "Failed to begin transaction for tenant allowlist delete");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model = match allowlist_queries::remove_tenant_allowlist_entry_in_tx(
        &tx, tenant_id, entry_id,
    )
    .await
    {
        Ok(Some(model)) => model,
        Ok(None) => {
            drop(tx);
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("discovery_allowlist_entry", entry_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "scope": "tenant",
                "reason_code": "allowlist_entry_not_found",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Allowlist entry not found");
        }
        Err(e) => {
            drop(tx);
            tracing::error!(error = %e, "DB error removing tenant discovery allowlist entry");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("discovery_allowlist_entry", entry_id.to_string(), None)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({
                "scope": "tenant",
                "reason_code": "tenant_discovery_allowlist_delete_failed",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_view = allowlist_queries::TenantAllowlistView::from(&before_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::discovery_allowlist_delete(
        &before_view,
        &AbsentView(&before_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "scope": "tenant",
        "plugin_type": before_model.plugin_type,
        "host_id": serde_json::Value::Null,
    }))
    .build()
    {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for tenant allowlist delete");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for tenant allowlist delete");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit tenant allowlist delete");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
}

// ── Host-specific endpoints ───────────────────────────────────────────────────

/// List host-specific discovery allowlist entries.
///
/// An empty list means the host inherits the tenant-wide allowlist (or all
/// plugins if the tenant list is also empty).
#[utoipa::path(
    get,
    path = "/api/v1/hosts/{id}/discovery-allowlist",
    params(("id" = Uuid, Path, description = "Host UUID")),
    responses(
        (status = 200, description = "Host-specific discovery allowlist entries", body = Vec<HostDiscoveryAllowlistEntry>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["software:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_host_discovery_allowlist(
    tenant_db: TenantDb,
    CanReadSoftware(_user): CanReadSoftware,
    Path(host_id): Path<Uuid>,
) -> Response {
    // Verify host belongs to tenant.
    match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!(error = %e, "DB error checking host");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    match allowlist_queries::list_host_allowlist(tenant_db.db(), tenant_db.tenant_id(), host_id)
        .await
    {
        Ok(entries) => (StatusCode::OK, Json(entries)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to list host discovery allowlist");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Add a plugin type to a host's discovery allowlist.
///
/// Once any entry is added for this host, only those plugin types run discovery
/// for this specific host. Host entries completely override the tenant-wide
/// allowlist for this host. Adding a duplicate entry returns the existing entry
/// with 201.
#[utoipa::path(
    post,
    path = "/api/v1/hosts/{id}/discovery-allowlist",
    params(("id" = Uuid, Path, description = "Host UUID")),
    request_body = CreateDiscoveryAllowlistEntryRequest,
    responses(
        (status = 201, description = "Entry created (or existing entry returned)", body = HostDiscoveryAllowlistEntry),
        (status = 400, description = "Invalid or non-discovery plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn add_host_discovery_allowlist_entry(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(host_id): Path<Uuid>,
    body: Unvalidated<CreateDiscoveryAllowlistEntryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({
                "scope": "host",
                "host_id": host_id,
                "reason_code": "invalid_request",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
        }
    };
    let plugin_type_str = req.plugin_type.to_string();

    // Verify host belongs to tenant (read-only, outside the tx).
    match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "scope": "host",
                "host_id": host_id,
                "plugin_type": plugin_type_str,
                "reason_code": "host_not_found",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return Ok(error_response(StatusCode::NOT_FOUND, "Host not found"));
        }
        Err(e) => {
            tracing::error!(error = %e, "DB error checking host");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({
                "scope": "host",
                "host_id": host_id,
                "plugin_type": plugin_type_str,
                "reason_code": "host_lookup_failed",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
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
            tracing::error!(error = %e, "Failed to begin transaction for host allowlist create");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let (was_created, entry) = match allowlist_queries::add_host_allowlist_entry_in_tx(
        state.plugin.plugin_ops.as_ref(),
        &tx,
        tenant_id,
        host_id,
        req.plugin_type,
    )
    .await
    {
        Ok(result) => result,
        Err(report) => {
            drop(tx);
            let (outcome, reason_code) =
                report.current_context().host_create_audit_classification();
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(outcome)
            .details(serde_json::json!({
                "scope": "host",
                "host_id": host_id,
                "plugin_type": plugin_type_str,
                "reason_code": reason_code,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return Err(report.into());
        }
    };

    if !was_created {
        // Entry already existed — no state change.
        drop(tx);
        if let Ok(audit_entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target(
            "discovery_allowlist_entry",
            entry.id.to_string(),
            Some(entry.plugin_type.clone()),
        )
        .outcome(AuditOutcome::Partial)
        .details(serde_json::json!({
            "scope": "host",
            "host_id": entry.host_id,
            "plugin_type": entry.plugin_type,
            "was_created": false,
        }))
        .build()
        {
            state.audit_emitter.emit_event(audit_entry);
        }
        return Ok((StatusCode::CREATED, Json(entry)).into_response());
    }

    // Newly inserted — emit_stateful.
    let view = allowlist_queries::HostAllowlistView::from(&entry);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::discovery_allowlist_create(
        &AbsentView(&view),
        &view,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "scope": "host",
        "host_id": entry.host_id,
        "plugin_type": entry.plugin_type,
        "was_created": true,
    }))
    .build()
    {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for host allowlist create");
            drop(tx);
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for host allowlist create");
        drop(tx);
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit host allowlist create");
        return Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ));
    }
    hook.flush_after_commit().await;

    Ok((StatusCode::CREATED, Json(entry)).into_response())
}

/// Remove a host-specific discovery allowlist entry.
///
/// Removing all host-specific entries makes this host inherit the tenant-wide
/// allowlist again.
#[utoipa::path(
    delete,
    path = "/api/v1/hosts/{id}/discovery-allowlist/{entry_id}",
    params(
        ("id" = Uuid, Path, description = "Host UUID"),
        ("entry_id" = Uuid, Path, description = "Allowlist entry UUID")
    ),
    responses(
        (status = 204, description = "Entry removed"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Entry not found")
    ),
    tag = "Autodiscovery",
    security(("oauth2" = ["software:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn remove_host_discovery_allowlist_entry(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((host_id, entry_id)): Path<(Uuid, Uuid)>,
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
            tracing::error!(error = %e, "Failed to begin transaction for host allowlist delete");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model = match allowlist_queries::remove_host_allowlist_entry_in_tx(
        &tx, tenant_id, host_id, entry_id,
    )
    .await
    {
        Ok(Some(model)) => model,
        Ok(None) => {
            drop(tx);
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("discovery_allowlist_entry", entry_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "scope": "host",
                "host_id": host_id,
                "reason_code": "allowlist_entry_not_found",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Allowlist entry not found");
        }
        Err(e) => {
            drop(tx);
            tracing::error!(error = %e, "DB error removing host discovery allowlist entry");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("discovery_allowlist_entry", entry_id.to_string(), None)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({
                "scope": "host",
                "host_id": host_id,
                "reason_code": "host_discovery_allowlist_delete_failed",
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_view = allowlist_queries::HostAllowlistView::from(&before_model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::discovery_allowlist_delete(
        &before_view,
        &AbsentView(&before_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "scope": "host",
        "host_id": host_id,
        "plugin_type": before_model.plugin_type,
    }))
    .build()
    {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for host allowlist delete");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for host allowlist delete");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit host allowlist delete");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::{insert_host, register_and_get_token};
    use http::StatusCode;
    use sea_orm::QueryOrder;
    use serde_json::Value;
    use uptrakit_shared_db::entity::audit_log;

    async fn latest_tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
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

    #[tokio::test]
    async fn add_tenant_discovery_allowlist_writes_create_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = register_and_get_token(&client).await;

        let (status, body): (StatusCode, Value) = client
            .post_json(
                "/api/v1/discovery-allowlist",
                &serde_json::json!({
                    "plugin_type": "package-manager.apt",
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, StatusCode::CREATED);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.target_type.as_deref(),
            Some("discovery_allowlist_entry")
        );
        assert_eq!(row.target_id.as_deref(), body["id"].as_str());
        assert_eq!(row.target_display.as_deref(), Some("package-manager.apt"));
        let details = row.details_json.expect("details");
        assert_eq!(details["scope"], serde_json::json!("tenant"));
        assert_eq!(
            details["plugin_type"],
            serde_json::json!("package-manager.apt")
        );
        assert_eq!(details["host_id"], Value::Null);
        assert_eq!(details["was_created"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn add_tenant_discovery_allowlist_invalid_plugin_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = register_and_get_token(&client).await;

        let status = client
            .post_json(
                "/api/v1/discovery-allowlist",
                &serde_json::json!({
                    "plugin_type": "releases.github",
                }),
            )
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(row.target_type, None);
        let details = row.details_json.expect("details");
        assert_eq!(details["scope"], serde_json::json!("tenant"));
        assert_eq!(details["plugin_type"], serde_json::json!("releases.github"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_plugin_type")
        );
    }

    #[tokio::test]
    async fn add_host_discovery_allowlist_writes_create_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = register_and_get_token(&client).await;
        let host = insert_host(&app.db, app.tenant_id).await;

        let (status, body): (StatusCode, Value) = client
            .post_json(
                &format!("/api/v1/hosts/{}/discovery-allowlist", host.id),
                &serde_json::json!({
                    "plugin_type": "package-manager.homebrew",
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(status, StatusCode::CREATED);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.target_type.as_deref(),
            Some("discovery_allowlist_entry")
        );
        assert_eq!(row.target_id.as_deref(), body["id"].as_str());
        assert_eq!(
            row.target_display.as_deref(),
            Some("package-manager.homebrew")
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["scope"], serde_json::json!("host"));
        assert_eq!(
            details["plugin_type"],
            serde_json::json!("package-manager.homebrew")
        );
        assert_eq!(details["host_id"], serde_json::json!(host.id));
        assert_eq!(details["was_created"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn remove_host_discovery_allowlist_writes_delete_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = register_and_get_token(&client).await;
        let host = insert_host(&app.db, app.tenant_id).await;

        let (create_status, create_body): (StatusCode, Value) = client
            .post_json(
                &format!("/api/v1/hosts/{}/discovery-allowlist", host.id),
                &serde_json::json!({
                    "plugin_type": "package-manager.apt",
                }),
            )
            .bearer(&token)
            .send_json()
            .await;
        assert_eq!(create_status, StatusCode::CREATED);
        let entry_id = create_body["id"].as_str().expect("entry id");

        let status = client
            .delete(&format!(
                "/api/v1/hosts/{}/discovery-allowlist/{}",
                host.id, entry_id
            ))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_id.as_deref(), Some(entry_id));
        let details = row.details_json.expect("details");
        assert_eq!(details["scope"], serde_json::json!("host"));
        assert_eq!(details["host_id"], serde_json::json!(host.id));
        assert_eq!(
            details["plugin_type"],
            serde_json::json!("package-manager.apt")
        );
    }

    #[tokio::test]
    async fn remove_tenant_discovery_allowlist_missing_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let token = register_and_get_token(&client).await;
        let missing_id = Uuid::now_v7();

        let status = client
            .delete(&format!("/api/v1/discovery-allowlist/{missing_id}"))
            .bearer(&token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let missing_id_str = missing_id.to_string();

        let row = latest_tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.target_id.as_deref(), Some(missing_id_str.as_str()));
        let details = row.details_json.expect("details");
        assert_eq!(details["scope"], serde_json::json!("tenant"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("allowlist_entry_not_found")
        );
    }
}
