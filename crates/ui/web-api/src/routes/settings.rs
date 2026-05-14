use crate::AppState;
use crate::auth::registration::RegistrationMode;
use crate::error_response::error_response;
use crate::extractors::{IfMatch, SettingsVersion};
use crate::middleware::permission::{CanManageAuthSettings, CanViewSettings};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_web_api_queries::queries::tenant_settings::TenantSettingView;

pub use uptrakit_web_api_types::settings::{
    RegistrationSettingsResponse, UpdateRegistrationSettingsRequest,
};

/// Emit a validation failure or persistence failure audit event for registration
/// settings (no DB write — uses the event dispatcher).
fn emit_registration_settings_event(
    state: &AppState,
    actor_type: uptrakit_audit_log::AuditActorType,
    actor_id: Option<uuid::Uuid>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    if let Ok(entry) = AuditEntry::<Event>::builder_event(
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
        state.audit_emitter.emit_event(entry);
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
    _if_match: IfMatch<SettingsVersion>,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<UpdateRegistrationSettingsRequest>,
) -> Response {
    let api_token_id_inner = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id_inner);

    // Validate: invite mode requires a token
    if req.mode == RegistrationMode::Invite && req.token.is_none() {
        emit_registration_settings_event(
            &state,
            actor_type,
            actor_id,
            AuditOutcome::ValidationFailed,
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

    // Capture before-state for the audit view.
    let before_reg = state.settings.registration();
    let before_view = TenantSettingView {
        key: "registration".to_string(),
        value: serde_json::json!({
            "mode": before_reg.mode.as_str(),
            "require_token_for_oidc": before_reg.require_token_for_oidc,
        }),
    };
    let had_existing_settings = before_reg.token_hash.is_some()
        || before_reg.mode != RegistrationMode::Closed
        || before_reg.require_token_for_oidc;

    let tenant_id = state.default_tenant_id;

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
            tracing::error!("Failed to begin tx for registration settings update: {e}");
            emit_registration_settings_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::Failed,
                serde_json::json!({
                    "setting_area": "registration",
                    "reason_code": "registration_settings_update_failed",
                    "mode_is_invite": mode_is_invite,
                    "token_provided": token_provided,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let hook = state.audit_emitter.commit_hook();

    let mut reg = state.settings.registration();
    if let Err(e) = reg
        .update(
            &tx,
            tenant_id,
            req.mode,
            req.token.map(|t| t.expose_secret().to_string()),
            req.require_token_for_oidc,
        )
        .await
    {
        tracing::error!(error = ?e, "Failed to update registration settings");
        drop(tx);
        emit_registration_settings_event(
            &state,
            actor_type,
            actor_id,
            AuditOutcome::Failed,
            serde_json::json!({
                "setting_area": "registration",
                "reason_code": "registration_settings_update_failed",
                "mode_is_invite": mode_is_invite,
                "token_provided": token_provided,
            }),
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    let after_view = TenantSettingView {
        key: "registration".to_string(),
        value: serde_json::json!({
            "mode": reg.mode.as_str(),
            "require_token_for_oidc": reg.require_token_for_oidc,
        }),
    };

    let audit_entry_result = if had_existing_settings {
        AuditEntry::<Stateful>::tenant_setting_update(&before_view, &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({
                "setting_area": "registration",
                "mode_is_invite": mode_is_invite,
                "require_token_for_oidc": reg.require_token_for_oidc,
                "token_provided": token_provided,
            }))
            .build()
    } else {
        AuditEntry::<Stateful>::tenant_setting_update(&AbsentView(&after_view), &after_view)
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({
                "setting_area": "registration",
                "mode_is_invite": mode_is_invite,
                "require_token_for_oidc": reg.require_token_for_oidc,
                "token_provided": token_provided,
            }))
            .build()
    };

    let audit_entry = match audit_entry_result {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for registration settings update: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit stateful audit for registration settings update: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit registration settings update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    state.settings.set_registration(reg).await;

    let reg = state.settings.registration();
    let response = RegistrationSettingsResponse {
        mode: reg.mode,
        require_token_for_oidc: reg.require_token_for_oidc,
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
            .header("if-match", "W/\"settings-v0\"")
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
            .header("if-match", "W/\"settings-v0\"")
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
