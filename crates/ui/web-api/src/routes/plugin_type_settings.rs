use crate::AppState;
use crate::app_state::PluginOpsState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::action::{AccessAuthority, CanManageSystemSettings, authorize_any};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::plugin_type_settings as pts_queries;
use crate::queries::plugin_type_settings::PluginTypeSettingsView;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{FromRequestParts, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_controller_core::access::{AccessContext, AccessEngine};
use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_shared_types::PluginTypeId;
use uptrakit_shared_types::access::{DenyReason, actions};
use uptrakit_web_api_types::plugin_type_settings::{
    PluginTypeSettingsResponse, UpsertPluginTypeSettingsRequest,
};

/// Convert a `plugin_type_setting::Model` into the API response type.
fn model_to_response(
    model: uptrakit_shared_db::entity::plugin_type_setting::Model,
) -> PluginTypeSettingsResponse {
    PluginTypeSettingsResponse {
        plugin_type: PluginTypeId::new(model.plugin_type),
        config: model.config,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

#[expect(
    clippy::result_large_err,
    reason = "error variant carries a Response which is large but unavoidable at this API boundary"
)]
fn validate_type_settings_payload(
    plugin_ops: &dyn PluginOps,
    plugin_type: &PluginTypeId,
    config: &serde_json::Value,
) -> Result<(), (&'static str, Response)> {
    if plugin_ops.get(plugin_type).is_none() {
        return Err((
            "unknown_plugin_type",
            error_response(StatusCode::BAD_REQUEST, "Unknown plugin type"),
        ));
    }

    if !plugin_ops.has_type_settings(plugin_type) {
        return Err((
            "plugin_type_settings_unsupported",
            error_response(
                StatusCode::BAD_REQUEST,
                format!(
                    "Plugin type '{}' does not support type settings",
                    plugin_type
                ),
            ),
        ));
    }

    if let Err(e) = plugin_ops.validate_config(plugin_type, config) {
        return Err((
            "plugin_type_settings_invalid",
            error_response(
                StatusCode::BAD_REQUEST,
                format!("Invalid plugin type settings: {e}"),
            ),
        ));
    }

    Ok(())
}

fn can_view_type_settings(engine: &AccessEngine, ctx: &AccessContext) -> Result<(), DenyReason> {
    authorize_any(
        engine,
        ctx,
        &[actions::SETTINGS_READ, actions::SYSTEM_SETTINGS_MANAGE],
    )
}

/// List all plugin type settings for the current tenant.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-type-settings",
    responses(
        (status = 200, description = "List of plugin type settings", body = Vec<PluginTypeSettingsResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(
        ("oauth2" = ["settings:read"]),
        ("oauth2" = ["system.settings:manage"]),
        ("developer_token" = [])
    )
)]
#[tracing::instrument(skip_all)]
pub async fn list_plugin_type_settings(
    State(state): State<Arc<AppState>>,
    State(plugin_ops): State<PluginOpsState>,
    tenant_db: TenantDb,
    Extension(authority): Extension<AccessAuthority>,
) -> Response {
    let Some(access_ctx) = authority.ready() else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    };
    if can_view_type_settings(&state.access_engine, access_ctx).is_err() {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    match pts_queries::list_type_settings(tenant_db.db(), tenant_db.tenant_id()).await {
        Ok(models) => {
            let snapshot = state.instance_plugin_snapshot.load_full();
            let responses: Vec<PluginTypeSettingsResponse> = models
                .into_iter()
                .filter(|m| {
                    plugin_ops
                        .0
                        .get(&PluginTypeId::new(&m.plugin_type))
                        .map(|d| {
                            crate::visibility::is_plugin_visible_to_user(
                                d,
                                plugin_ops.0.as_ref(),
                                snapshot.as_ref(),
                                &state.access_engine,
                                access_ctx,
                            )
                        })
                        .unwrap_or(false)
                })
                .map(model_to_response)
                .collect();
            (StatusCode::OK, Json(responses)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to list plugin type settings");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get plugin type settings for a specific plugin type.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-type-settings/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    responses(
        (status = 200, description = "Plugin type settings", body = PluginTypeSettingsResponse),
        (status = 404, description = "No settings found for this plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(
        ("oauth2" = ["settings:read"]),
        ("oauth2" = ["system.settings:manage"]),
        ("developer_token" = [])
    )
)]
#[tracing::instrument(skip_all)]
pub async fn get_plugin_type_settings(
    State(state): State<Arc<AppState>>,
    State(plugin_ops): State<PluginOpsState>,
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    Extension(authority): Extension<AccessAuthority>,
) -> Response {
    let Some(access_ctx) = authority.ready() else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    };
    if can_view_type_settings(&state.access_engine, access_ctx).is_err() {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let plugin_type_id = PluginTypeId::new(&plugin_type);
    if let Some(desc) = plugin_ops.0.get(&plugin_type_id)
        && !crate::visibility::is_plugin_visible_to_user(
            desc,
            plugin_ops.0.as_ref(),
            state.instance_plugin_snapshot.load().as_ref(),
            &state.access_engine,
            access_ctx,
        )
    {
        return error_response(
            StatusCode::NOT_FOUND,
            "No settings found for this plugin type",
        );
    }

    match pts_queries::get_type_settings(tenant_db.db(), tenant_db.tenant_id(), &plugin_type).await
    {
        Ok(Some(model)) => (StatusCode::OK, Json(model_to_response(model))).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "No settings found for this plugin type",
        ),
        Err(e) => {
            tracing::error!(error = %e, "Failed to get plugin type settings");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Bundles the API-token identity and access-authority extensions needed by
/// state-mutating plugin-type-settings handlers, keeping the handler's own
/// argument list under clippy's `too_many_arguments` threshold.
///
/// Same technique as `routes/surfaces.rs`'s `GetInteractionRequest` — a
/// `FromRequestParts` bundle standing in for several extractors — though
/// that one groups `Method` + raw query rather than the auth extensions.
pub struct WriteAuthContext {
    api_token_id: Option<AuthenticatedApiTokenId>,
    authority: AccessAuthority,
}

impl FromRequestParts<Arc<AppState>> for WriteAuthContext {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let api_token_id = Extension::<AuthenticatedApiTokenId>::from_request_parts(parts, state)
            .await
            .ok()
            .map(|Extension(id)| id);
        let Extension(authority) = Extension::<AccessAuthority>::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(Self {
            api_token_id,
            authority,
        })
    }
}

/// Create or update plugin type settings for a specific plugin type.
///
/// If settings already exist for the given plugin type, they are updated.
/// Otherwise, new settings are created.
#[utoipa::path(
    put,
    path = "/api/v1/plugin-type-settings/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    request_body = UpsertPluginTypeSettingsRequest,
    responses(
        (status = 200, description = "Plugin type settings created or updated", body = PluginTypeSettingsResponse),
        (status = 400, description = "Invalid input"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("oauth2" = ["system.settings:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn upsert_plugin_type_settings(
    State(state): State<Arc<AppState>>,
    State(plugin_ops): State<PluginOpsState>,
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    CanManageSystemSettings(user): CanManageSystemSettings,
    write_ctx: WriteAuthContext,
    Validated(req): Validated<UpsertPluginTypeSettingsRequest>,
) -> Response {
    let WriteAuthContext {
        api_token_id,
        authority,
    } = write_ctx;
    let Some(access_ctx) = authority.ready() else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    };
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();
    let plugin_type_id = PluginTypeId::new(&plugin_type);

    if let Some(desc) = plugin_ops.0.get(&plugin_type_id)
        && !crate::visibility::is_plugin_visible_to_user(
            desc,
            plugin_ops.0.as_ref(),
            state.instance_plugin_snapshot.load().as_ref(),
            &state.access_engine,
            access_ctx,
        )
    {
        return error_response(StatusCode::NOT_FOUND, "Unknown plugin type");
    }

    if let Err((reason_code, rejection)) =
        validate_type_settings_payload(plugin_ops.0.as_ref(), &plugin_type_id, &req.config)
    {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target(
            "plugin_type_settings",
            plugin_type.clone(),
            Some(plugin_type.clone()),
        )
        .outcome(AuditOutcome::ValidationFailed)
        .details(serde_json::json!({ "reason_code": reason_code }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return rejection;
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
            tracing::error!("Failed to begin transaction for plugin type settings upsert: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let (before_model, after_model) =
        match pts_queries::upsert_type_settings_in_tx(&tx, tenant_id, &plugin_type, req.config)
            .await
        {
            Ok(pair) => pair,
            Err(e) => {
                drop(tx);
                tracing::error!("Failed to upsert plugin type settings: {e}");
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target(
                    "plugin_type_settings",
                    plugin_type.clone(),
                    Some(plugin_type.clone()),
                )
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({
                    "reason_code": "plugin_type_settings_upsert_failed"
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let after_view = PluginTypeSettingsView::from(&after_model);
    let audit_entry_result = if let Some(ref before) = before_model {
        let before_view = PluginTypeSettingsView::from(before);
        AuditEntry::<Stateful>::plugin_type_settings_upsert(&before_view, &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({}))
            .build()
    } else {
        AuditEntry::<Stateful>::plugin_type_settings_upsert(&AbsentView(&after_view), &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({}))
            .build()
    };
    let audit_entry = match audit_entry_result {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for plugin type settings upsert: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hook = state.audit_emitter.commit_hook();
    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for plugin type settings upsert: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit plugin type settings upsert: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    (StatusCode::OK, Json(model_to_response(after_model))).into_response()
}

/// Delete plugin type settings, resetting to defaults.
#[utoipa::path(
    delete,
    path = "/api/v1/plugin-type-settings/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    responses(
        (status = 204, description = "Plugin type settings deleted (reset to defaults)"),
        (status = 404, description = "No settings found for this plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("oauth2" = ["system.settings:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_plugin_type_settings(
    State(state): State<Arc<AppState>>,
    State(plugin_ops): State<PluginOpsState>,
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    CanManageSystemSettings(user): CanManageSystemSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Extension(authority): Extension<AccessAuthority>,
) -> Response {
    let Some(access_ctx) = authority.ready() else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    };
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let plugin_type_id = PluginTypeId::new(&plugin_type);
    if let Some(desc) = plugin_ops.0.get(&plugin_type_id)
        && !crate::visibility::is_plugin_visible_to_user(
            desc,
            plugin_ops.0.as_ref(),
            state.instance_plugin_snapshot.load().as_ref(),
            &state.access_engine,
            access_ctx,
        )
    {
        return error_response(
            StatusCode::NOT_FOUND,
            "No settings found for this plugin type",
        );
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
            tracing::error!("Failed to begin transaction for plugin type settings delete: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model =
        match pts_queries::delete_type_settings_in_tx(&tx, tenant_id, &plugin_type).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                drop(tx);
                if let Ok(entry) = AuditEntry::<Event>::builder_event(
                    uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_DELETE,
                )
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .target(
                    "plugin_type_settings",
                    plugin_type.clone(),
                    Some(plugin_type.clone()),
                )
                .outcome(AuditOutcome::Denied)
                .details(serde_json::json!({
                    "reason_code": "plugin_type_settings_not_found"
                }))
                .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(
                    StatusCode::NOT_FOUND,
                    "No settings found for this plugin type",
                );
            }
            Err(e) => {
                drop(tx);
                tracing::error!("Failed to delete plugin type settings: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let before_view = PluginTypeSettingsView::from(&before_model);
    let audit_entry = match AuditEntry::<Stateful>::plugin_type_settings_delete(
        &before_view,
        &AbsentView(&before_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({}))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for plugin type settings delete: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hook = state.audit_emitter.commit_hook();
    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for plugin type settings delete: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit plugin type settings delete: {e}");
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
    use crate::test_harness::fixtures;
    use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::audit_log;

    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query tenant audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected tenant audit row for action {action_type}");
    }

    #[tokio::test]
    async fn upsert_plugin_type_settings_writes_semantic_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let plugin_type = "package-manager.cargo";
        let plugin_type_id = PluginTypeId::new(plugin_type);
        let config = app
            .state
            .plugin
            .plugin_ops
            .type_settings_sample(&plugin_type_id);

        let request = UpsertPluginTypeSettingsRequest { config };
        let status = client
            .put_json(
                &format!("/api/v1/plugin-type-settings/{plugin_type}"),
                &request,
            )
            .bearer(&access_token)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::OK);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("plugin_type_settings"));
        assert_eq!(row.target_id.as_deref(), Some(plugin_type));

        // First upsert — no prior settings: before_snapshot is {}
        let before = row.before_snapshot.expect("before_snapshot must be set");
        assert_eq!(before, serde_json::json!({}));
        // after_snapshot has plugin_type as the semantic identifier
        let after = row.after_snapshot.expect("after_snapshot must be set");
        assert_eq!(after["plugin_type"], serde_json::json!(plugin_type));
        // config must not appear in the snapshot
        assert!(
            after.get("config").is_none(),
            "raw config must not appear in after_snapshot"
        );
    }

    #[tokio::test]
    async fn upsert_unknown_plugin_type_writes_validation_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let status = client
            .put_json(
                "/api/v1/plugin-type-settings/not_a_real_plugin",
                &serde_json::json!({ "config": {} }),
            )
            .bearer(&access_token)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("unknown_plugin_type")
        );
    }

    #[tokio::test]
    async fn delete_missing_plugin_type_settings_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let plugin_type = "package-manager.cargo";

        let status = client
            .delete(&format!("/api/v1/plugin-type-settings/{plugin_type}"))
            .bearer(&access_token)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("plugin_type_settings_not_found")
        );
    }

    #[tokio::test]
    async fn upsert_plugin_type_settings_db_failure_writes_failed_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let plugin_type = "package-manager.cargo";
        let plugin_type_id = PluginTypeId::new(plugin_type);
        let config = app
            .state
            .plugin
            .plugin_ops
            .type_settings_sample(&plugin_type_id);

        app.db
            .execute_unprepared("DROP TABLE plugin_type_settings")
            .await
            .expect("drop plugin_type_settings table");

        let status = client
            .put_json(
                &format!("/api/v1/plugin-type-settings/{plugin_type}"),
                &UpsertPluginTypeSettingsRequest { config },
            )
            .bearer(&access_token)
            .send_status()
            .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("plugin_type_settings_upsert_failed")
        );
    }
}
