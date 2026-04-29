use crate::app_state::{AuditEmitterState, PluginOpsState};
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::plugin_type_settings as pts_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_shared_types::PluginTypeId;
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

#[allow(clippy::result_large_err)]
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

fn can_view_type_settings(user: &AuthenticatedUser) -> bool {
    user.has_permission(Permission::ViewSettings)
        || user.has_permission(Permission::ManageGlobalSettings)
}

struct AuditContext<'a> {
    audit_emitter: &'a uptrakit_audit_log::AuditEmitter,
    tenant_id: uuid::Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_plugin_type_settings_audit(
    ctx: &AuditContext<'_>,
    plugin_type: &str,
    operation: &'static str,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
    config_field_count: Option<usize>,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);
    let action_type = if operation == "delete" {
        uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_DELETE
    } else {
        uptrakit_audit_log::AuditActionType::PLUGIN_TYPE_SETTINGS_UPSERT
    };

    let mut details = serde_json::Map::from_iter([
        ("plugin_type".to_string(), serde_json::json!(plugin_type)),
        ("operation".to_string(), serde_json::json!(operation)),
        (
            "changed".to_string(),
            serde_json::json!(matches!(outcome, uptrakit_audit_log::AuditOutcome::Success)),
        ),
    ]);
    if let Some(reason_code) = reason_code {
        details.insert("reason_code".to_string(), serde_json::json!(reason_code));
    }
    if let Some(field_count) = config_field_count {
        details.insert(
            "config_field_count".to_string(),
            serde_json::json!(field_count),
        );
    }

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .target(
            "plugin_type_settings",
            plugin_type.to_string(),
            Some(plugin_type.to_string()),
        )
        .outcome(outcome)
        .details(serde_json::Value::Object(details))
        .build()
    {
        ctx.audit_emitter.emit_best_effort(entry);
    }
}

/// List all plugin type settings for the current tenant.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-type-settings",
    extensions(("x-required-permission" = json!(["view_settings", "manage_global_settings"]))),
    responses(
        (status = 200, description = "List of plugin type settings", body = Vec<PluginTypeSettingsResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_plugin_type_settings(
    tenant_db: TenantDb,
    Extension(auth_user): Extension<AuthenticatedUser>,
) -> Response {
    if !can_view_type_settings(&auth_user) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    match pts_queries::list_type_settings(tenant_db.db(), tenant_db.tenant_id).await {
        Ok(models) => {
            let responses: Vec<PluginTypeSettingsResponse> =
                models.into_iter().map(model_to_response).collect();
            (StatusCode::OK, Json(responses)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list plugin type settings: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get plugin type settings for a specific plugin type.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-type-settings/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    extensions(("x-required-permission" = json!(["view_settings", "manage_global_settings"]))),
    responses(
        (status = 200, description = "Plugin type settings", body = PluginTypeSettingsResponse),
        (status = 404, description = "No settings found for this plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_plugin_type_settings(
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    Extension(auth_user): Extension<AuthenticatedUser>,
) -> Response {
    if !can_view_type_settings(&auth_user) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    match pts_queries::get_type_settings(tenant_db.db(), tenant_db.tenant_id, &plugin_type).await {
        Ok(Some(model)) => (StatusCode::OK, Json(model_to_response(model))).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "No settings found for this plugin type",
        ),
        Err(e) => {
            tracing::error!("Failed to get plugin type settings: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
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
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, description = "Plugin type settings created or updated", body = PluginTypeSettingsResponse),
        (status = 400, description = "Invalid input"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn upsert_plugin_type_settings(
    State(audit_emitter_state): State<AuditEmitterState>,
    State(plugin_ops): State<PluginOpsState>,
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<UpsertPluginTypeSettingsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    let plugin_type_id = PluginTypeId::new(&plugin_type);
    let config_field_count = req.config.as_object().map(|v| v.len()).unwrap_or(0);
    if let Err((reason_code, rejection)) =
        validate_type_settings_payload(plugin_ops.0.as_ref(), &plugin_type_id, &req.config)
    {
        emit_plugin_type_settings_audit(
            &audit_ctx,
            &plugin_type,
            "upsert",
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            Some(reason_code),
            Some(config_field_count),
        );
        return rejection;
    }

    match pts_queries::upsert_type_settings(
        tenant_db.db(),
        tenant_db.tenant_id,
        &plugin_type,
        req.config,
    )
    .await
    {
        Ok(model) => {
            emit_plugin_type_settings_audit(
                &audit_ctx,
                &plugin_type,
                "upsert",
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                Some(config_field_count),
            );
            (StatusCode::OK, Json(model_to_response(model))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to upsert plugin type settings: {e}");
            emit_plugin_type_settings_audit(
                &audit_ctx,
                &plugin_type,
                "upsert",
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("plugin_type_settings_upsert_failed"),
                Some(config_field_count),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Delete plugin type settings, resetting to defaults.
#[utoipa::path(
    delete,
    path = "/api/v1/plugin-type-settings/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 204, description = "Plugin type settings deleted (reset to defaults)"),
        (status = 404, description = "No settings found for this plugin type"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Plugin Type Settings",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_plugin_type_settings(
    State(audit_emitter_state): State<AuditEmitterState>,
    tenant_db: TenantDb,
    Path(plugin_type): Path<String>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id,
        user: &user,
        api_token_id,
    };
    match pts_queries::delete_type_settings(tenant_db.db(), tenant_db.tenant_id, &plugin_type).await
    {
        Ok(true) => {
            emit_plugin_type_settings_audit(
                &audit_ctx,
                &plugin_type,
                "delete",
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                None,
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_plugin_type_settings_audit(
                &audit_ctx,
                &plugin_type,
                "delete",
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("plugin_type_settings_not_found"),
                None,
            );
            error_response(
                StatusCode::NOT_FOUND,
                "No settings found for this plugin type",
            )
        }
        Err(e) => {
            tracing::error!("Failed to delete plugin type settings: {e}");
            emit_plugin_type_settings_audit(
                &audit_ctx,
                &plugin_type,
                "delete",
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("plugin_type_settings_delete_failed"),
                None,
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
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

        let plugin_type = "package_manager_cargo";
        let plugin_type_id = PluginTypeId::new(plugin_type);
        let config = app.state.plugin_ops.type_settings_sample(&plugin_type_id);
        let config_field_count = config.as_object().map(|v| v.len()).unwrap_or(0);

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

        let details = row.details_json.expect("details");
        assert_eq!(details["plugin_type"], serde_json::json!(plugin_type));
        assert_eq!(details["operation"], serde_json::json!("upsert"));
        assert_eq!(
            details["config_field_count"],
            serde_json::json!(config_field_count)
        );
        assert!(
            details.get("config").is_none(),
            "raw config must not be present in audit details"
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
        assert_eq!(details["operation"], serde_json::json!("upsert"));
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
        let plugin_type = "package_manager_cargo";

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
        assert_eq!(details["operation"], serde_json::json!("delete"));
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

        let plugin_type = "package_manager_cargo";
        let plugin_type_id = PluginTypeId::new(plugin_type);
        let config = app.state.plugin_ops.type_settings_sample(&plugin_type_id);

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
        assert_eq!(details["operation"], serde_json::json!("upsert"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("plugin_type_settings_upsert_failed")
        );
    }
}
