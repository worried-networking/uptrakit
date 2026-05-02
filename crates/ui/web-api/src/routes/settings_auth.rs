use crate::AppState;
use crate::auth::AuthMethod;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
#[cfg(feature = "oidc")]
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
#[cfg(feature = "oidc")]
use {
    sea_orm::{ColumnTrait, QueryFilter},
    uptrakit_shared_db::entity::oidc_provider,
};

pub use uptrakit_web_api_types::settings_auth::{
    AuthenticationSettingsResponse, UpdateAuthenticationSettingsRequest,
};

fn emit_auth_settings_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(actor_type, actor_id)
    .target(
        "tenant_setting",
        "authentication".to_string(),
        Some("authentication".to_string()),
    )
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn auth_settings_audit_details(
    previous_enabled: bool,
    new_enabled: bool,
    reason_code: Option<&'static str>,
) -> serde_json::Value {
    let mut details = serde_json::json!({
        "setting_key": "authentication.password_auth_enabled",
        "previous_enabled": previous_enabled,
        "new_enabled": new_enabled,
        "changed": previous_enabled != new_enabled,
    });

    if let Some(reason_code) = reason_code
        && let Some(map) = details.as_object_mut()
    {
        map.insert(
            "reason_code".to_string(),
            serde_json::Value::String(reason_code.to_string()),
        );
    }

    details
}

/// Get authentication settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/authentication",
    responses(
        (status = 200, description = "Authentication settings", body = AuthenticationSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_authentication_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let auth_settings = state.settings.authentication();
    let response = AuthenticationSettingsResponse {
        password_auth_enabled: auth_settings.password_auth_enabled,
    };
    (StatusCode::OK, Json(response)).into_response()
}

/// Update authentication settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/authentication",
    request_body = UpdateAuthenticationSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = AuthenticationSettingsResponse),
        (status = 409, description = "Safety check failed")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_authentication_settings(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    #[cfg(feature = "oidc")] tenant_db: TenantDb,
    Json(req): Json<UpdateAuthenticationSettingsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);

    if let Some(password_enabled) = req.password_auth_enabled {
        let previous_enabled = state.settings.authentication().password_auth_enabled;
        if !password_enabled {
            // Safety: cannot disable password auth if current session uses password
            if user.auth_method == AuthMethod::Password {
                emit_auth_settings_audit(
                    &state,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    auth_settings_audit_details(
                        previous_enabled,
                        password_enabled,
                        Some("cannot_disable_password_auth_while_using_password"),
                    ),
                );
                return error_response(
                    StatusCode::CONFLICT,
                    "Cannot disable password authentication while logged in with a password",
                );
            }

            // Safety: at least one auth method must remain
            #[cfg(feature = "oidc")]
            {
                let active_providers = tenant_db
                    .find::<oidc_provider::Entity>()
                    .filter(oidc_provider::Column::IsActive.eq(true))
                    .filter(oidc_provider::Column::DeactivatedAt.is_null())
                    .all(tenant_db.db())
                    .await
                    .unwrap_or_default();

                if active_providers.is_empty() {
                    emit_auth_settings_audit(
                        &state,
                        &user,
                        api_token_id,
                        uptrakit_audit_log::AuditOutcome::Denied,
                        auth_settings_audit_details(
                            previous_enabled,
                            password_enabled,
                            Some("cannot_disable_password_auth_without_active_oidc_providers"),
                        ),
                    );
                    return error_response(
                        StatusCode::CONFLICT,
                        "Cannot disable password authentication with no active OIDC providers",
                    );
                }
            }

            if !cfg!(feature = "oidc") {
                emit_auth_settings_audit(
                    &state,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    auth_settings_audit_details(
                        previous_enabled,
                        password_enabled,
                        Some("cannot_disable_password_auth_without_oidc_support"),
                    ),
                );
                return error_response(
                    StatusCode::CONFLICT,
                    "Cannot disable password authentication: OIDC support is not enabled",
                );
            }
        }

        let mut auth_settings = state.settings.authentication();
        auth_settings.password_auth_enabled = password_enabled;
        if let Err(e) = auth_settings
            .save(state.db(), state.default_tenant_id)
            .await
        {
            tracing::error!("Failed to save authentication settings: {e:?}");
            emit_auth_settings_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                auth_settings_audit_details(
                    previous_enabled,
                    password_enabled,
                    Some("authentication_settings_update_failed"),
                ),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state.settings.set_authentication(auth_settings).await;

        emit_auth_settings_audit(
            &state,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditOutcome::Success,
            auth_settings_audit_details(previous_enabled, password_enabled, None),
        );
    }

    let auth_settings = state.settings.authentication();
    let response = AuthenticationSettingsResponse {
        password_auth_enabled: auth_settings.password_auth_enabled,
    };
    (StatusCode::OK, Json(response)).into_response()
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
    async fn update_authentication_settings_safety_conflict_writes_denied_tenant_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let status = client
            .put_json(
                "/api/v1/settings/authentication",
                &serde_json::json!({
                    "password_auth_enabled": false
                }),
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("tenant_setting"));
        assert_eq!(row.target_id.as_deref(), Some("authentication"));
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("cannot_disable_password_auth_while_using_password")
        );
    }

    #[tokio::test]
    async fn update_authentication_settings_save_failure_writes_failed_tenant_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        app.db
            .execute_unprepared("DROP TABLE settings")
            .await
            .expect("drop settings table");

        let status = client
            .put_json(
                "/api/v1/settings/authentication",
                &serde_json::json!({
                    "password_auth_enabled": true
                }),
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("authentication_settings_update_failed")
        );
    }
}
