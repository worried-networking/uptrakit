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
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageSoftware, CanViewSoftware};
use crate::queries::discovery_allowlist as allowlist_queries;
use crate::tenant_db::TenantDb;
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
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Tenant-wide discovery allowlist entries", body = Vec<TenantDiscoveryAllowlistEntry>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn list_tenant_discovery_allowlist(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
) -> Response {
    match allowlist_queries::list_tenant_allowlist(tenant_db.db(), tenant_db.tenant_id).await {
        Ok(entries) => (StatusCode::OK, Json(entries)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list tenant discovery allowlist: {e}");
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 201, description = "Entry created (or existing entry returned)", body = TenantDiscoveryAllowlistEntry),
        (status = 400, description = "Invalid or non-discovery plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn add_tenant_discovery_allowlist_entry(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Json(req): Json<CreateDiscoveryAllowlistEntryRequest>,
) -> Response {
    match allowlist_queries::add_tenant_allowlist_entry(
        state.plugin_ops.as_ref(),
        tenant_db.db(),
        tenant_db.tenant_id,
        req.plugin_type,
    )
    .await
    {
        Ok(entry) => (StatusCode::CREATED, Json(entry)).into_response(),
        Err(report) => match report.current_context() {
            allowlist_queries::AllowlistError::InvalidPluginType => error_response(
                StatusCode::BAD_REQUEST,
                "plugin type does not support discovery or is unknown",
            ),
            allowlist_queries::AllowlistError::Db(_) => {
                tracing::error!("DB error adding tenant discovery allowlist entry: {report}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        },
    }
}

/// Remove a tenant-wide discovery allowlist entry.
///
/// Removing all entries restores the "unconfigured = all allowed" default.
#[utoipa::path(
    delete,
    path = "/api/v1/discovery-allowlist/{id}",
    params(("id" = Uuid, Path, description = "Allowlist entry UUID")),
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 204, description = "Entry removed"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Entry not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn remove_tenant_discovery_allowlist_entry(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(entry_id): Path<Uuid>,
) -> Response {
    match allowlist_queries::remove_tenant_allowlist_entry(
        tenant_db.db(),
        tenant_db.tenant_id,
        entry_id,
    )
    .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Allowlist entry not found"),
        Err(e) => {
            tracing::error!("DB error removing tenant discovery allowlist entry: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
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
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Host-specific discovery allowlist entries", body = Vec<HostDiscoveryAllowlistEntry>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn list_host_discovery_allowlist(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Path(host_id): Path<Uuid>,
) -> Response {
    // Verify host belongs to tenant.
    match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error checking host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    match allowlist_queries::list_host_allowlist(tenant_db.db(), tenant_db.tenant_id, host_id).await
    {
        Ok(entries) => (StatusCode::OK, Json(entries)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list host discovery allowlist: {e}");
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 201, description = "Entry created (or existing entry returned)", body = HostDiscoveryAllowlistEntry),
        (status = 400, description = "Invalid or non-discovery plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Host not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn add_host_discovery_allowlist_entry(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path(host_id): Path<Uuid>,
    Json(req): Json<CreateDiscoveryAllowlistEntryRequest>,
) -> Response {
    // Verify host belongs to tenant.
    match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error checking host: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    match allowlist_queries::add_host_allowlist_entry(
        state.plugin_ops.as_ref(),
        tenant_db.db(),
        tenant_db.tenant_id,
        host_id,
        req.plugin_type,
    )
    .await
    {
        Ok(entry) => (StatusCode::CREATED, Json(entry)).into_response(),
        Err(report) => match report.current_context() {
            allowlist_queries::AllowlistError::InvalidPluginType => error_response(
                StatusCode::BAD_REQUEST,
                "plugin type does not support discovery or is unknown",
            ),
            allowlist_queries::AllowlistError::Db(_) => {
                tracing::error!("DB error adding host discovery allowlist entry: {report}");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        },
    }
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
    extensions(("x-required-permission" = json!("manage_software"))),
    responses(
        (status = 204, description = "Entry removed"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Entry not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
pub async fn remove_host_discovery_allowlist_entry(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((host_id, entry_id)): Path<(Uuid, Uuid)>,
) -> Response {
    match allowlist_queries::remove_host_allowlist_entry(
        tenant_db.db(),
        tenant_db.tenant_id,
        host_id,
        entry_id,
    )
    .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Allowlist entry not found"),
        Err(e) => {
            tracing::error!("DB error removing host discovery allowlist entry: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
