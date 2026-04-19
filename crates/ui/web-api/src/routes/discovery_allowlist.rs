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
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::AppState;
use crate::api_error::ApiError;
use crate::error_response::error_response;
use crate::middleware::permission::{CanUpdateSoftware, CanViewSoftware};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::discovery_allowlist as allowlist_queries;
use crate::tenant_db::TenantDb;
use uptrakit_shared_db::entity::{
    host, host_discovery_allowlist, prelude::*, tenant_discovery_allowlist,
};
use uptrakit_web_api_queries::queries::discovery_allowlist::AllowlistError;

pub use uptrakit_web_api_types::discovery_allowlist::{
    CreateDiscoveryAllowlistEntryRequest, HostDiscoveryAllowlistEntry,
    TenantDiscoveryAllowlistEntry,
};

fn emit_discovery_allowlist_audit(
    state: &AppState,
    tenant_id: Uuid,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    action_type: &'static str,
    target: Option<(Uuid, Option<String>)>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);
    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(details);

    if let Some((entry_id, target_display)) = target {
        builder = builder.target(
            "discovery_allowlist_entry",
            entry_id.to_string(),
            target_display,
        );
    }

    if let Ok(entry) = builder.build() {
        state.audit_emitter.emit_best_effort(entry);
    }
}

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
#[tracing::instrument(skip_all)]
pub async fn list_tenant_discovery_allowlist(
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
) -> Response {
    match allowlist_queries::list_tenant_allowlist(tenant_db.db(), tenant_db.tenant_id).await {
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
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 201, description = "Entry created (or existing entry returned)", body = TenantDiscoveryAllowlistEntry),
        (status = 400, description = "Invalid or non-discovery plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn add_tenant_discovery_allowlist_entry(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<CreateDiscoveryAllowlistEntryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let plugin_type = req.plugin_type.to_string();
    let was_created = tenant_discovery_allowlist::Entity::find()
        .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(tenant_discovery_allowlist::Column::PluginType.eq(&plugin_type))
        .one(tenant_db.db())
        .await
        .map(|row| row.is_none())
        .unwrap_or(true);

    let entry = match allowlist_queries::add_tenant_allowlist_entry(
        state.plugin_ops.as_ref(),
        tenant_db.db(),
        tenant_db.tenant_id,
        req.plugin_type,
    )
    .await
    {
        Ok(entry) => entry,
        Err(report) => {
            let (outcome, reason_code) = match report.current_context() {
                AllowlistError::InvalidPluginType => (
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    "invalid_plugin_type",
                ),
                AllowlistError::Db(_) => (
                    uptrakit_audit_log::AuditOutcome::Failed,
                    "tenant_discovery_allowlist_create_failed",
                ),
            };
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
                None,
                outcome,
                serde_json::json!({
                    "scope": "tenant",
                    "plugin_type": plugin_type,
                    "reason_code": reason_code,
                }),
            );
            return Err(report.into());
        }
    };

    emit_discovery_allowlist_audit(
        &state,
        tenant_db.tenant_id,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
        Some((entry.id, Some(entry.plugin_type.clone()))),
        if was_created {
            uptrakit_audit_log::AuditOutcome::Success
        } else {
            uptrakit_audit_log::AuditOutcome::Partial
        },
        serde_json::json!({
            "scope": "tenant",
            "plugin_type": entry.plugin_type,
            "host_id": serde_json::Value::Null,
            "was_created": was_created,
        }),
    );
    Ok((StatusCode::CREATED, Json(entry)).into_response())
}

/// Remove a tenant-wide discovery allowlist entry.
///
/// Removing all entries restores the "unconfigured = all allowed" default.
#[utoipa::path(
    delete,
    path = "/api/v1/discovery-allowlist/{id}",
    params(("id" = Uuid, Path, description = "Allowlist entry UUID")),
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 204, description = "Entry removed"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Entry not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
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
    let existing_entry = match tenant_discovery_allowlist::Entity::find_by_id(entry_id)
        .filter(tenant_discovery_allowlist::Column::TenantId.eq(tenant_db.tenant_id))
        .one(tenant_db.db())
        .await
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "DB error loading tenant discovery allowlist entry");
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
                Some((entry_id, None)),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "scope": "tenant",
                    "reason_code": "tenant_discovery_allowlist_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match allowlist_queries::remove_tenant_allowlist_entry(
        tenant_db.db(),
        tenant_db.tenant_id,
        entry_id,
    )
    .await
    {
        Ok(true) => {
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
                Some((
                    entry_id,
                    existing_entry
                        .as_ref()
                        .map(|entry| entry.plugin_type.clone()),
                )),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "scope": "tenant",
                    "plugin_type": existing_entry.as_ref().map(|entry| entry.plugin_type.clone()),
                    "host_id": serde_json::Value::Null,
                }),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
                Some((entry_id, existing_entry.map(|entry| entry.plugin_type))),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "scope": "tenant",
                    "reason_code": "allowlist_entry_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Allowlist entry not found")
        }
        Err(e) => {
            tracing::error!(error = %e, "DB error removing tenant discovery allowlist entry");
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
                Some((entry_id, existing_entry.map(|entry| entry.plugin_type))),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "scope": "tenant",
                    "reason_code": "tenant_discovery_allowlist_delete_failed",
                }),
            );
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
#[tracing::instrument(skip_all)]
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
            tracing::error!(error = %e, "DB error checking host");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    match allowlist_queries::list_host_allowlist(tenant_db.db(), tenant_db.tenant_id, host_id).await
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
    extensions(("x-required-permission" = json!("update_software"))),
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
#[tracing::instrument(skip_all)]
pub async fn add_host_discovery_allowlist_entry(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateSoftware(user): CanUpdateSoftware,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(host_id): Path<Uuid>,
    Json(req): Json<CreateDiscoveryAllowlistEntryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let plugin_type = req.plugin_type.to_string();

    // Verify host belongs to tenant.
    match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "scope": "host",
                    "host_id": host_id,
                    "plugin_type": plugin_type,
                    "reason_code": "host_not_found",
                }),
            );
            return Ok(error_response(StatusCode::NOT_FOUND, "Host not found"));
        }
        Err(e) => {
            tracing::error!(error = %e, "DB error checking host");
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "scope": "host",
                    "host_id": host_id,
                    "plugin_type": plugin_type,
                    "reason_code": "host_lookup_failed",
                }),
            );
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    }

    let was_created = host_discovery_allowlist::Entity::find()
        .filter(host_discovery_allowlist::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host_discovery_allowlist::Column::HostId.eq(host_id))
        .filter(host_discovery_allowlist::Column::PluginType.eq(&plugin_type))
        .one(tenant_db.db())
        .await
        .map(|row| row.is_none())
        .unwrap_or(true);

    let entry = match allowlist_queries::add_host_allowlist_entry(
        state.plugin_ops.as_ref(),
        tenant_db.db(),
        tenant_db.tenant_id,
        host_id,
        req.plugin_type,
    )
    .await
    {
        Ok(entry) => entry,
        Err(report) => {
            let (outcome, reason_code) = match report.current_context() {
                AllowlistError::InvalidPluginType => (
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    "invalid_plugin_type",
                ),
                AllowlistError::Db(_) => (
                    uptrakit_audit_log::AuditOutcome::Failed,
                    "host_discovery_allowlist_create_failed",
                ),
            };
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
                None,
                outcome,
                serde_json::json!({
                    "scope": "host",
                    "host_id": host_id,
                    "plugin_type": plugin_type,
                    "reason_code": reason_code,
                }),
            );
            return Err(report.into());
        }
    };

    emit_discovery_allowlist_audit(
        &state,
        tenant_db.tenant_id,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_CREATE,
        Some((entry.id, Some(entry.plugin_type.clone()))),
        if was_created {
            uptrakit_audit_log::AuditOutcome::Success
        } else {
            uptrakit_audit_log::AuditOutcome::Partial
        },
        serde_json::json!({
            "scope": "host",
            "host_id": entry.host_id,
            "plugin_type": entry.plugin_type,
            "was_created": was_created,
        }),
    );
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
    extensions(("x-required-permission" = json!("update_software"))),
    responses(
        (status = 204, description = "Entry removed"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Entry not found")
    ),
    tag = "Autodiscovery",
    security(("bearer_token" = []))
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
    let existing_entry = match host_discovery_allowlist::Entity::find_by_id(entry_id)
        .filter(host_discovery_allowlist::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host_discovery_allowlist::Column::HostId.eq(host_id))
        .one(tenant_db.db())
        .await
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "DB error loading host discovery allowlist entry");
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
                Some((entry_id, None)),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "scope": "host",
                    "host_id": host_id,
                    "reason_code": "host_discovery_allowlist_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match allowlist_queries::remove_host_allowlist_entry(
        tenant_db.db(),
        tenant_db.tenant_id,
        host_id,
        entry_id,
    )
    .await
    {
        Ok(true) => {
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
                Some((
                    entry_id,
                    existing_entry
                        .as_ref()
                        .map(|entry| entry.plugin_type.clone()),
                )),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "scope": "host",
                    "host_id": host_id,
                    "plugin_type": existing_entry.as_ref().map(|entry| entry.plugin_type.clone()),
                }),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
                Some((entry_id, existing_entry.map(|entry| entry.plugin_type))),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "scope": "host",
                    "host_id": host_id,
                    "reason_code": "allowlist_entry_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Allowlist entry not found")
        }
        Err(e) => {
            tracing::error!(error = %e, "DB error removing host discovery allowlist entry");
            emit_discovery_allowlist_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::DISCOVERY_ALLOWLIST_DELETE,
                Some((entry_id, existing_entry.map(|entry| entry.plugin_type))),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "scope": "host",
                    "host_id": host_id,
                    "reason_code": "host_discovery_allowlist_delete_failed",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures::{
        insert_host, register_and_get_token, seed_permissions_for_owner,
    };
    use http::StatusCode;
    use sea_orm::QueryOrder;
    use serde_json::Value;
    use uptrakit_shared_db::entity::audit_log;

    async fn latest_tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: &'static str,
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
        seed_permissions_for_owner(&app.db, &["update_software"]).await;
        let token = register_and_get_token(&client).await;

        let (status, body): (StatusCode, Value) = client
            .post_json(
                "/api/v1/discovery-allowlist",
                &serde_json::json!({
                    "plugin_type": "package_manager_apt",
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
        assert_eq!(row.target_display.as_deref(), Some("package_manager_apt"));
        let details = row.details_json.expect("details");
        assert_eq!(details["scope"], serde_json::json!("tenant"));
        assert_eq!(
            details["plugin_type"],
            serde_json::json!("package_manager_apt")
        );
        assert_eq!(details["host_id"], Value::Null);
        assert_eq!(details["was_created"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn add_tenant_discovery_allowlist_invalid_plugin_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["update_software"]).await;
        let token = register_and_get_token(&client).await;

        let status = client
            .post_json(
                "/api/v1/discovery-allowlist",
                &serde_json::json!({
                    "plugin_type": "releases_github",
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
        assert_eq!(details["plugin_type"], serde_json::json!("releases_github"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_plugin_type")
        );
    }

    #[tokio::test]
    async fn add_host_discovery_allowlist_writes_create_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["update_software"]).await;
        let token = register_and_get_token(&client).await;
        let host = insert_host(&app.db, app.tenant_id).await;

        let (status, body): (StatusCode, Value) = client
            .post_json(
                &format!("/api/v1/hosts/{}/discovery-allowlist", host.id),
                &serde_json::json!({
                    "plugin_type": "package_manager_homebrew",
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
            Some("package_manager_homebrew")
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["scope"], serde_json::json!("host"));
        assert_eq!(
            details["plugin_type"],
            serde_json::json!("package_manager_homebrew")
        );
        assert_eq!(details["host_id"], serde_json::json!(host.id));
        assert_eq!(details["was_created"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn remove_host_discovery_allowlist_writes_delete_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["update_software"]).await;
        let token = register_and_get_token(&client).await;
        let host = insert_host(&app.db, app.tenant_id).await;

        let (create_status, create_body): (StatusCode, Value) = client
            .post_json(
                &format!("/api/v1/hosts/{}/discovery-allowlist", host.id),
                &serde_json::json!({
                    "plugin_type": "package_manager_apt",
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
            serde_json::json!("package_manager_apt")
        );
    }

    #[tokio::test]
    async fn remove_tenant_discovery_allowlist_missing_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        seed_permissions_for_owner(&app.db, &["update_software"]).await;
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
