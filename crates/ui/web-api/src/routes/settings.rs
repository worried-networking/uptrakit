use crate::AppState;
use crate::auth::registration::RegistrationMode;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub use uptrakit_web_api_types::settings::{
    RegistrationSettingsResponse, UpdateRegistrationSettingsRequest,
};

fn emit_registration_settings_audit(
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
        "registration".to_string(),
        Some("registration".to_string()),
    )
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

/// Get current registration settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/registration",
    responses(
        (status = 200, description = "Current registration settings", body = RegistrationSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_registration_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    let reg = state.settings.registration();
    let response = RegistrationSettingsResponse {
        mode: reg.mode,
        require_token_for_oidc: reg.require_token_for_oidc,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Update registration settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/registration",
    request_body = UpdateRegistrationSettingsRequest,
    responses(
        (status = 200, description = "Registration settings updated", body = RegistrationSettingsResponse),
        (status = 400, description = "Invalid request (e.g., invite mode without token)"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_auth_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_registration_settings(
    State(state): State<Arc<AppState>>,
    CanManageAuthSettings(user): CanManageAuthSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<UpdateRegistrationSettingsRequest>,
) -> Response {
    // Validate: invite mode requires a token
    if req.mode == RegistrationMode::Invite && req.token.is_none() {
        emit_registration_settings_audit(
            &state,
            &user,
            api_token_id.map(|value| value.0),
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "setting_area": "registration",
                "reason_code": "invite_mode_requires_token",
                "mode_is_invite": true,
                "token_provided": false,
            }),
        );
        return error_response(
            StatusCode::BAD_REQUEST,
            "Token is required when mode is invite",
        );
    }

    let token_provided = req.token.is_some();
    let mode_is_invite = req.mode == RegistrationMode::Invite;

    let mut reg = state.settings.registration();
    if let Err(e) = reg
        .update(
            state.db(),
            state.default_tenant_id,
            req.mode,
            req.token.map(|t| t.expose_secret().to_string()),
            req.require_token_for_oidc,
        )
        .await
    {
        tracing::error!(error = ?e, "Failed to update registration settings");
        emit_registration_settings_audit(
            &state,
            &user,
            api_token_id.map(|value| value.0),
            uptrakit_audit_log::AuditOutcome::Failed,
            serde_json::json!({
                "setting_area": "registration",
                "reason_code": "registration_settings_update_failed",
                "mode_is_invite": mode_is_invite,
                "token_provided": token_provided,
            }),
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    state.settings.set_registration(reg).await;

    let reg = state.settings.registration();
    let response = RegistrationSettingsResponse {
        mode: reg.mode,
        require_token_for_oidc: reg.require_token_for_oidc,
    };

    emit_registration_settings_audit(
        &state,
        &user,
        api_token_id.map(|value| value.0),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "setting_area": "registration",
            "mode_is_invite": mode_is_invite,
            "require_token_for_oidc": reg.require_token_for_oidc,
            "token_provided": token_provided,
        }),
    );

    (StatusCode::OK, Json(response)).into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
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
    async fn update_registration_settings_writes_tenant_setting_update_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let secret_token = "very-secret-registration-token";

        let status = client
            .put_json(
                "/api/v1/settings/registration",
                &serde_json::json!({
                    "mode": "invite",
                    "token": secret_token,
                    "require_token_for_oidc": true
                }),
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
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
        assert_eq!(row.target_type.as_deref(), Some("tenant_setting"));
        assert_eq!(row.target_id.as_deref(), Some("registration"));

        let details = row.details_json.expect("details");
        assert_eq!(details["setting_area"], serde_json::json!("registration"));
        assert_eq!(details["token_provided"], serde_json::json!(true));
        assert!(
            !details.to_string().contains(secret_token),
            "registration token must never be present in audit details"
        );
    }

    #[tokio::test]
    async fn update_registration_settings_validation_failure_writes_tenant_setting_update_audit_event()
     {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let status = client
            .put_json(
                "/api/v1/settings/registration",
                &serde_json::json!({
                    "mode": "invite",
                    "require_token_for_oidc": true
                }),
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = tenant_audit_row_for_action(
            &app.db,
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["setting_area"], serde_json::json!("registration"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invite_mode_requires_token")
        );
        assert_eq!(details["token_provided"], serde_json::json!(false));
    }
}
