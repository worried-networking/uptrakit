use crate::AppState;
use crate::SettingKey;
use crate::error_response::error_response;
use crate::extract::Unvalidated;
use crate::middleware::action::{CanManageSettingsCertificates, CanReadSettings};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::settings_store::{delete_setting, upsert_setting};
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_web_api_queries::queries::tenant_settings::TenantSettingView;

pub use uptrakit_web_api_types::settings_agent_certs::{
    AgentCertificateSettingsResponse, UpdateAgentCertificateSettingsRequest,
};

const MAX_AGENT_CERT_LIFETIME_HOURS: u32 = 17_520;

/// Emit a validation failure or persistence failure audit event for agent certificate
/// settings (no DB write — uses the event dispatcher).
fn emit_agent_cert_settings_event(
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
        "agent_certificates".to_string(),
        Some("agent_certificates".to_string()),
    )
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_event(entry);
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
    security(("oauth2" = ["settings:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_agent_certificate_settings(
    State(state): State<Arc<AppState>>,
    CanReadSettings(_user): CanReadSettings,
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
    security(("oauth2" = ["settings.certificates:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_agent_certificate_settings(
    State(state): State<Arc<AppState>>,
    CanManageSettingsCertificates(user): CanManageSettingsCertificates,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    body: Unvalidated<UpdateAgentCertificateSettingsRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = state.default_tenant_id;

    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            emit_agent_cert_settings_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "setting_area": "agent_certificates",
                    "reason_code": "invalid_request",
                }),
            );
            return error_response(StatusCode::BAD_REQUEST, e.to_string());
        }
    };
    let mut changed_keys = Vec::new();
    let mut renewal_window_reset_to_auto = false;

    // Snapshot before-state for the combined audit view.
    let before_lifetime = state.settings.agent_cert_lifetime_hours();
    let before_renewal_window = state.settings.renewal_window_hours_override();

    if let Some(hours) = req.lifetime_hours {
        if hours < 1 {
            emit_agent_cert_settings_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::ValidationFailed,
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
            emit_agent_cert_settings_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::ValidationFailed,
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
                tracing::error!("Failed to begin tx for agent cert lifetime update: {e}");
                emit_agent_cert_settings_event(
                    &state,
                    actor_type,
                    actor_id,
                    AuditOutcome::Failed,
                    serde_json::json!({
                        "setting_area": "agent_certificates",
                        "reason_code": "agent_cert_lifetime_upsert_failed",
                        "setting_key": SettingKey::AgentCertLifetimeHours.as_str(),
                        "provided_lifetime_hours": hours,
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        let hook = state.audit_emitter.commit_hook();

        if let Err(e) = upsert_setting(
            &tx,
            tenant_id,
            SettingKey::AgentCertLifetimeHours,
            serde_json::json!(hours),
        )
        .await
        {
            tracing::error!("Failed to save agent cert lifetime: {e:?}");
            drop(tx);
            emit_agent_cert_settings_event(
                &state,
                actor_type,
                actor_id,
                AuditOutcome::Failed,
                serde_json::json!({
                    "setting_area": "agent_certificates",
                    "reason_code": "agent_cert_lifetime_upsert_failed",
                    "setting_key": SettingKey::AgentCertLifetimeHours.as_str(),
                    "provided_lifetime_hours": hours,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }

        let before_view = TenantSettingView {
            key: SettingKey::AgentCertLifetimeHours.as_str().to_string(),
            value: serde_json::json!(before_lifetime),
        };
        let after_view = TenantSettingView {
            key: SettingKey::AgentCertLifetimeHours.as_str().to_string(),
            value: serde_json::json!(hours),
        };
        let audit_entry =
            match AuditEntry::<Stateful>::tenant_setting_update(&before_view, &after_view)
                .tenant_scope(tenant_id)
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Success)
                .details(serde_json::json!({
                    "setting_area": "agent_certificates",
                    "setting_key": SettingKey::AgentCertLifetimeHours.as_str(),
                    "provided_lifetime_hours": hours,
                }))
                .build()
            {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::error!(
                        "Failed to build audit entry for agent cert lifetime update: {e}"
                    );
                    drop(tx);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

        if let Err(e) = state
            .audit_emitter
            .emit_stateful(&tx, &hook, audit_entry)
            .await
        {
            tracing::error!("Failed to emit stateful audit for agent cert lifetime update: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }

        if let Err(e) = tx.commit().await {
            tracing::error!("Failed to commit agent cert lifetime update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
        hook.flush_after_commit().await;

        state.settings.set_agent_cert_lifetime_hours(hours).await;
        changed_keys.push(SettingKey::AgentCertLifetimeHours.as_str());
    }

    if let Some(hours) = req.renewal_window_hours {
        if hours == 0 {
            // Reset to automatic mode: remove the override from the DB.
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
                    tracing::error!("Failed to begin tx for agent cert renewal window delete: {e}");
                    emit_agent_cert_settings_event(
                        &state,
                        actor_type,
                        actor_id,
                        AuditOutcome::Failed,
                        serde_json::json!({
                            "setting_area": "agent_certificates",
                            "reason_code": "agent_cert_renewal_window_delete_failed",
                            "setting_key": SettingKey::AgentCertRenewalWindowHours.as_str(),
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };
            let hook = state.audit_emitter.commit_hook();

            if let Err(e) =
                delete_setting(&tx, tenant_id, SettingKey::AgentCertRenewalWindowHours).await
            {
                tracing::error!("Failed to delete renewal window setting: {e:?}");
                drop(tx);
                emit_agent_cert_settings_event(
                    &state,
                    actor_type,
                    actor_id,
                    AuditOutcome::Failed,
                    serde_json::json!({
                        "setting_area": "agent_certificates",
                        "reason_code": "agent_cert_renewal_window_delete_failed",
                        "setting_key": SettingKey::AgentCertRenewalWindowHours.as_str(),
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }

            let before_view = TenantSettingView {
                key: SettingKey::AgentCertRenewalWindowHours.as_str().to_string(),
                value: serde_json::json!(before_renewal_window),
            };
            let after_view = TenantSettingView {
                key: SettingKey::AgentCertRenewalWindowHours.as_str().to_string(),
                value: serde_json::Value::Null,
            };
            let audit_entry =
                match AuditEntry::<Stateful>::tenant_setting_update(&before_view, &after_view)
                    .tenant_scope(tenant_id)
                    .actor(actor_type, actor_id)
                    .outcome(AuditOutcome::Success)
                    .details(serde_json::json!({
                        "setting_area": "agent_certificates",
                        "setting_key": SettingKey::AgentCertRenewalWindowHours.as_str(),
                        "renewal_window_reset_to_auto": true,
                    }))
                    .build()
                {
                    Ok(entry) => entry,
                    Err(e) => {
                        tracing::error!(
                            "Failed to build audit entry for agent cert renewal window delete: {e}"
                        );
                        drop(tx);
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                };

            if let Err(e) = state
                .audit_emitter
                .emit_stateful(&tx, &hook, audit_entry)
                .await
            {
                tracing::error!(
                    "Failed to emit stateful audit for agent cert renewal window delete: {e}"
                );
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }

            if let Err(e) = tx.commit().await {
                tracing::error!("Failed to commit agent cert renewal window delete: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            hook.flush_after_commit().await;

            state.settings.set_renewal_window_hours_override(None).await;
            renewal_window_reset_to_auto = true;
        } else {
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
                    tracing::error!("Failed to begin tx for agent cert renewal window upsert: {e}");
                    emit_agent_cert_settings_event(
                        &state,
                        actor_type,
                        actor_id,
                        AuditOutcome::Failed,
                        serde_json::json!({
                            "setting_area": "agent_certificates",
                            "reason_code": "agent_cert_renewal_window_upsert_failed",
                            "setting_key": SettingKey::AgentCertRenewalWindowHours.as_str(),
                            "provided_renewal_window_hours": hours,
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };
            let hook = state.audit_emitter.commit_hook();

            if let Err(e) = upsert_setting(
                &tx,
                tenant_id,
                SettingKey::AgentCertRenewalWindowHours,
                serde_json::json!(hours),
            )
            .await
            {
                tracing::error!("Failed to save renewal window: {e:?}");
                drop(tx);
                emit_agent_cert_settings_event(
                    &state,
                    actor_type,
                    actor_id,
                    AuditOutcome::Failed,
                    serde_json::json!({
                        "setting_area": "agent_certificates",
                        "reason_code": "agent_cert_renewal_window_upsert_failed",
                        "setting_key": SettingKey::AgentCertRenewalWindowHours.as_str(),
                        "provided_renewal_window_hours": hours,
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }

            let before_view = TenantSettingView {
                key: SettingKey::AgentCertRenewalWindowHours.as_str().to_string(),
                value: serde_json::json!(before_renewal_window),
            };
            let after_view = TenantSettingView {
                key: SettingKey::AgentCertRenewalWindowHours.as_str().to_string(),
                value: serde_json::json!(hours),
            };
            let audit_entry =
                match AuditEntry::<Stateful>::tenant_setting_update(&before_view, &after_view)
                    .tenant_scope(tenant_id)
                    .actor(actor_type, actor_id)
                    .outcome(AuditOutcome::Success)
                    .details(serde_json::json!({
                        "setting_area": "agent_certificates",
                        "setting_key": SettingKey::AgentCertRenewalWindowHours.as_str(),
                        "provided_renewal_window_hours": hours,
                    }))
                    .build()
                {
                    Ok(entry) => entry,
                    Err(e) => {
                        tracing::error!(
                            "Failed to build audit entry for agent cert renewal window upsert: {e}"
                        );
                        drop(tx);
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                };

            if let Err(e) = state
                .audit_emitter
                .emit_stateful(&tx, &hook, audit_entry)
                .await
            {
                tracing::error!(
                    "Failed to emit stateful audit for agent cert renewal window upsert: {e}"
                );
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }

            if let Err(e) = tx.commit().await {
                tracing::error!("Failed to commit agent cert renewal window upsert: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            hook.flush_after_commit().await;

            state
                .settings
                .set_renewal_window_hours_override(Some(hours))
                .await;
        }
        changed_keys.push(SettingKey::AgentCertRenewalWindowHours.as_str());
    }

    // The `changed_keys` and `renewal_window_reset_to_auto` variables are
    // retained here for potential future use (e.g. a combined summary event).
    // Individual per-key stateful audit entries are already emitted above.
    let _ = changed_keys;
    let _ = renewal_window_reset_to_auto;

    (StatusCode::OK, Json(build_response(&state))).into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::auth::AuthMethod;
    use crate::middleware::action::CanManageSettingsCertificates;
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
            CanManageSettingsCertificates::new(AuthenticatedUser::new(
                user_id,
                AuthMethod::Password,
                None,
            )),
            None,
            crate::extract::Unvalidated::new_for_test(UpdateAgentCertificateSettingsRequest {
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
            CanManageSettingsCertificates::new(AuthenticatedUser::new(
                uuid::Uuid::now_v7(),
                AuthMethod::Password,
                None,
            )),
            None,
            crate::extract::Unvalidated::new_for_test(UpdateAgentCertificateSettingsRequest {
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
