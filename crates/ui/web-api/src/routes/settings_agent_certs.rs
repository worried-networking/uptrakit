use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::middleware::permission::{CanManageAgentCerts, CanViewSettings};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::settings_store::{delete_setting, upsert_setting};
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub use uptrakit_web_api_types::settings_agent_certs::{
    AgentCertificateSettingsResponse, UpdateAgentCertificateSettingsRequest,
};

const MAX_AGENT_CERT_LIFETIME_HOURS: u32 = 17_520;

fn emit_agent_cert_settings_audit(
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
        "agent_certificates".to_string(),
        Some("agent_certificates".to_string()),
    )
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn build_response(state: &AppState) -> AgentCertificateSettingsResponse {
    AgentCertificateSettingsResponse {
        lifetime_hours: state.settings.agent_cert_lifetime_hours(),
        renewal_window_hours_override: state.settings.renewal_window_hours_override(),
        effective_renewal_window_hours: state.settings.renewal_window_hours(),
    }
}

/// Get agent certificate settings
#[utoipa::path(
    get,
    path = "/api/v1/settings/agent-certificates",
    responses(
        (status = 200, description = "Agent certificate settings", body = AgentCertificateSettingsResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("view_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_agent_certificate_settings(
    State(state): State<Arc<AppState>>,
    CanViewSettings(_user): CanViewSettings,
) -> Response {
    (StatusCode::OK, Json(build_response(&state))).into_response()
}

/// Update agent certificate settings
#[utoipa::path(
    put,
    path = "/api/v1/settings/agent-certificates",
    request_body = UpdateAgentCertificateSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = AgentCertificateSettingsResponse),
        (status = 400, description = "Invalid values"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_agent_certs"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_agent_certificate_settings(
    State(state): State<Arc<AppState>>,
    CanManageAgentCerts(user): CanManageAgentCerts,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<UpdateAgentCertificateSettingsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let mut changed_keys = Vec::new();
    let mut renewal_window_reset_to_auto = false;

    if let Some(hours) = req.lifetime_hours {
        if hours < 1 {
            emit_agent_cert_settings_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "setting_area": "agent_certificates",
                    "reason_code": "agent_cert_lifetime_below_minimum",
                    "setting_key": SettingKey::AgentCertLifetimeHours.as_str(),
                    "provided_lifetime_hours": hours,
                    "minimum_lifetime_hours": 1,
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                "Certificate lifetime must be at least 1 hour",
            );
        }
        if hours > MAX_AGENT_CERT_LIFETIME_HOURS {
            emit_agent_cert_settings_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "setting_area": "agent_certificates",
                    "reason_code": "agent_cert_lifetime_exceeds_maximum",
                    "setting_key": SettingKey::AgentCertLifetimeHours.as_str(),
                    "provided_lifetime_hours": hours,
                    "maximum_lifetime_hours": MAX_AGENT_CERT_LIFETIME_HOURS,
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                "Certificate lifetime must not exceed 17520 hours",
            );
        }
        if let Err(e) = upsert_setting(
            state.db(),
            state.default_tenant_id,
            SettingKey::AgentCertLifetimeHours,
            serde_json::json!(hours),
        )
        .await
        {
            tracing::error!("Failed to save agent cert lifetime: {e:?}");
            emit_agent_cert_settings_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "setting_area": "agent_certificates",
                    "reason_code": "agent_cert_lifetime_upsert_failed",
                    "setting_key": SettingKey::AgentCertLifetimeHours.as_str(),
                    "provided_lifetime_hours": hours,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        state.settings.set_agent_cert_lifetime_hours(hours).await;
        changed_keys.push(SettingKey::AgentCertLifetimeHours.as_str());
    }

    if let Some(hours) = req.renewal_window_hours {
        if hours == 0 {
            // Reset to automatic mode: remove the override from the DB.
            if let Err(e) = delete_setting(
                state.db(),
                state.default_tenant_id,
                SettingKey::AgentCertRenewalWindowHours,
            )
            .await
            {
                tracing::error!("Failed to delete renewal window setting: {e:?}");
                emit_agent_cert_settings_audit(
                    &state,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "setting_area": "agent_certificates",
                        "reason_code": "agent_cert_renewal_window_delete_failed",
                        "setting_key": SettingKey::AgentCertRenewalWindowHours.as_str(),
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            state.settings.set_renewal_window_hours_override(None).await;
            renewal_window_reset_to_auto = true;
        } else {
            if let Err(e) = upsert_setting(
                state.db(),
                state.default_tenant_id,
                SettingKey::AgentCertRenewalWindowHours,
                serde_json::json!(hours),
            )
            .await
            {
                tracing::error!("Failed to save renewal window: {e:?}");
                emit_agent_cert_settings_audit(
                    &state,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "setting_area": "agent_certificates",
                        "reason_code": "agent_cert_renewal_window_upsert_failed",
                        "setting_key": SettingKey::AgentCertRenewalWindowHours.as_str(),
                        "provided_renewal_window_hours": hours,
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            state
                .settings
                .set_renewal_window_hours_override(Some(hours))
                .await;
        }
        changed_keys.push(SettingKey::AgentCertRenewalWindowHours.as_str());
    }

    if !changed_keys.is_empty() {
        emit_agent_cert_settings_audit(
            &state,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditOutcome::Success,
            serde_json::json!({
                "setting_area": "agent_certificates",
                "changed_keys": changed_keys,
                "renewal_window_reset_to_auto": renewal_window_reset_to_auto,
            }),
        );
    }

    (StatusCode::OK, Json(build_response(&state))).into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::middleware::permission::CanManageAgentCerts;
    use crate::middleware::require_auth::AuthenticatedUser;
    use sea_orm::{
        ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, PaginatorTrait,
        QueryFilter, QueryOrder,
    };
    use uptrakit_shared_db::entity::audit_log;

    async fn latest_tenant_setting_update_audit_row(db: &DatabaseConnection) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(
                    audit_log::Column::ActionType
                        .eq(uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE),
                )
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query tenant audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant setting update audit row");
    }

    async fn wait_for_tenant_audit_rows(db: &DatabaseConnection, expected: u64) {
        for _ in 0..50 {
            let count = audit_log::Entity::find()
                .count(db)
                .await
                .expect("count tenant audit rows");
            if count == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected {expected} tenant audit rows");
    }

    #[tokio::test]
    async fn update_agent_certificate_settings_validation_failure_writes_tenant_setting_update_audit_event()
     {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let user_id = uuid::Uuid::now_v7();
        let response = update_agent_certificate_settings(
            State(Arc::clone(&state)),
            CanManageAgentCerts::new(AuthenticatedUser {
                user_id,
                auth_method: AuthMethod::Password,
                permissions: vec![Permission::ManageAgentCerts],
            }),
            None,
            Json(UpdateAgentCertificateSettingsRequest {
                lifetime_hours: Some(0),
                renewal_window_hours: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        wait_for_tenant_audit_rows(&db, 1).await;
        let row = latest_tenant_setting_update_audit_row(&db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id));
        assert_eq!(row.target_type.as_deref(), Some("tenant_setting"));
        assert_eq!(row.target_id.as_deref(), Some("agent_certificates"));
        let details = row.details_json.expect("details");
        assert_eq!(
            details["setting_area"],
            serde_json::json!("agent_certificates")
        );
        assert_eq!(
            details["reason_code"],
            serde_json::json!("agent_cert_lifetime_below_minimum")
        );
        assert_eq!(details["provided_lifetime_hours"], serde_json::json!(0));
    }

    #[tokio::test]
    async fn update_agent_certificate_settings_persistence_failure_writes_tenant_setting_update_audit_event()
     {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (mut state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let raw_db = Database::connect(ConnectOptions::new("sqlite::memory:".to_string()))
            .await
            .expect("raw sqlite db");
        Arc::make_mut(&mut state).db = crate::app_state::DbState::new(raw_db);

        let response = update_agent_certificate_settings(
            State(Arc::clone(&state)),
            CanManageAgentCerts::new(AuthenticatedUser {
                user_id: uuid::Uuid::now_v7(),
                auth_method: AuthMethod::Password,
                permissions: vec![Permission::ManageAgentCerts],
            }),
            None,
            Json(UpdateAgentCertificateSettingsRequest {
                lifetime_hours: Some(24),
                renewal_window_hours: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        wait_for_tenant_audit_rows(&db, 1).await;
        let row = latest_tenant_setting_update_audit_row(&db).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["setting_area"],
            serde_json::json!("agent_certificates")
        );
        assert_eq!(
            details["reason_code"],
            serde_json::json!("agent_cert_lifetime_upsert_failed")
        );
        assert_eq!(
            details["setting_key"],
            serde_json::json!(SettingKey::AgentCertLifetimeHours.as_str())
        );
    }
}
