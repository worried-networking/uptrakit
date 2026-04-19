use crate::AppState;
use crate::actions::settings as settings_actions;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_web_api_types::settings_reset::{ResetDataRequest, ResetDataResponse};

fn emit_reset_data_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::TENANT_DATA_RESET,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(actor_type, actor_id)
    .target("tenant", state.default_tenant_id.to_string(), None)
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

/// Reset all tenant-scoped data (hosts, software items, configs, history, etc.)
#[utoipa::path(
    post,
    path = "/api/v1/settings/reset-data",
    request_body = ResetDataRequest,
    responses(
        (status = 200, description = "Data reset successfully", body = ResetDataResponse),
        (status = 400, description = "Invalid request (confirm != RESET)"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Settings",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn reset_data(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    tenant_db: TenantDb,
    Validated(_req): Validated<ResetDataRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let ctx = state.mutation_context();
    match settings_actions::reset_data(&tenant_db, &ctx, &state.service_connections).await {
        Ok(counts) => {
            emit_reset_data_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "deleted_total": counts.hosts
                        + counts.software_items
                        + counts.plugin_configs
                        + counts.host_tags
                        + counts.update_history
                        + counts.update_batches,
                }),
            );
            (StatusCode::OK, Json(ResetDataResponse { deleted: counts })).into_response()
        }
        Err(e) => {
            tracing::error!("failed to reset tenant data: {:?}", e);
            emit_reset_data_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "tenant_data_reset_failed",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

#[cfg(all(test, feature = "db-sqlite", feature = "reset-data"))]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::middleware::permission::CanManageGlobalSettings;
    use crate::middleware::require_auth::AuthenticatedUser;
    use sea_orm::{
        ColumnTrait, ConnectOptions, Database, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    };
    use uptrakit_shared_db::entity::audit_log;

    async fn tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: &str,
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

    async fn wait_for_tenant_audit_rows(db: &sea_orm::DatabaseConnection, expected: u64) {
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
    async fn reset_data_failure_writes_failed_tenant_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let raw_db = Database::connect(ConnectOptions::new("sqlite::memory:".to_string()))
            .await
            .expect("raw sqlite db");
        let tenant_db = TenantDb::new_for_test(raw_db, tenant_id);

        let user_id = uuid::Uuid::now_v7();
        let response = reset_data(
            State(Arc::clone(&state)),
            CanManageGlobalSettings::new(AuthenticatedUser {
                user_id,
                auth_method: AuthMethod::Password,
                permissions: vec![Permission::ManageGlobalSettings],
            }),
            None,
            tenant_db,
            Validated(ResetDataRequest {
                confirm: "RESET".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        wait_for_tenant_audit_rows(&db, 1).await;
        let row = tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::TENANT_DATA_RESET,
        )
        .await;
        assert_eq!(
            row.action_type,
            uptrakit_audit_log::AuditActionType::TENANT_DATA_RESET
        );
        assert_eq!(row.tenant_id, tenant_id);
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("tenant_data_reset_failed")
        );
    }
}
