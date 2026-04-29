use std::sync::Arc;

use axum::response::IntoResponse;
use axum::{Extension, extract::State};
use http::StatusCode;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};

pub use uptrakit_web_api_types::settings_ca::RotateCaResponse;

fn emit_rotate_ca_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id, actor_display) = authenticated_user_audit_actor(user, api_token_id);

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SYSTEM_CA_ROTATE,
    )
    .system_scope()
    .actor(actor_type, actor_id)
    .actor_display_opt(actor_display)
    .target(
        "certificate_authority",
        "controller_ca".to_string(),
        Some("controller_ca".to_string()),
    )
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

/// Trigger an immediate CA rotation.
///
/// Signals the CA rotation background task to execute immediately.
/// After rotation, the controller broadcasts `CaBundleUpdated` and
/// `RequestCertRenewal` to all connected agents.
///
/// Requires authentication (handled by the `require_auth` layer).
#[utoipa::path(
    post,
    path = "/api/v1/global-settings/ca/rotate",
    responses(
        (status = 200, description = "CA rotation triggered", body = RotateCaResponse),
        (status = 400, description = "CA rotation not available"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Global Settings",
    extensions(("x-required-permission" = json!("manage_global_settings")))
)]
#[tracing::instrument(skip_all)]
pub async fn rotate_ca(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> impl IntoResponse {
    let api_token_id = api_token_id.map(|value| value.0);
    let snapshot = state.cert.ca_snapshot.borrow().clone();
    if !snapshot.managed {
        emit_rotate_ca_audit(
            &state,
            &user,
            api_token_id,
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "ca_rotation_not_available_for_unmanaged_ca",
            }),
        );
        return error_response(
            StatusCode::BAD_REQUEST,
            "CA rotation is only available for managed (internally generated) CAs",
        );
    }

    // Signal the CA rotation background task to run immediately
    state.cert.ca_rotation_trigger.notify_one();

    // Dispatch notification event for CA rotation.
    state.notification.notification_dispatcher.dispatch(
        crate::notifications::events::NotificationEvent {
            tenant_id: state.default_tenant_id,
            host_id: None,
            host_name: None,
            software_item_id: None,
            software_item_name: None,
            plugin_type: None,
            details: crate::notifications::events::NotificationEventDetails::CaRotated {
                reason: "manual rotation via API".to_string(),
            },
        },
    );

    emit_rotate_ca_audit(
        &state,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "triggered_by": "api",
            "managed_ca": true,
        }),
    );

    (
        StatusCode::OK,
        axum::Json(RotateCaResponse {
            message: "CA rotation triggered. Connected agents will be notified to renew their certificates.".to_string(),
        }),
    )
        .into_response()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::middleware::permission::CanManageGlobalSettings;
    use crate::middleware::require_auth::AuthenticatedUser;
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::system_audit_log;

    async fn latest_system_audit_row(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> system_audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = system_audit_log::Entity::find()
                .filter(system_audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected system audit row for action {action_type}");
    }

    async fn wait_for_system_audit_rows(db: &sea_orm::DatabaseConnection, expected: u64) {
        for _ in 0..50 {
            let count = system_audit_log::Entity::find()
                .count(db)
                .await
                .expect("count system audit rows");
            if count == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected {expected} system audit rows");
    }

    #[tokio::test]
    async fn rotate_ca_writes_success_system_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let user_id = uuid::Uuid::now_v7();
        let response = rotate_ca(
            State(Arc::clone(&state)),
            CanManageGlobalSettings::new(AuthenticatedUser {
                user_id,
                auth_method: AuthMethod::Password,
                permissions: vec![Permission::ManageGlobalSettings],
                jti: None,
                actor_display: None,
            }),
            None,
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        wait_for_system_audit_rows(&db, 1).await;
        let row =
            latest_system_audit_row(&db, uptrakit_audit_log::AuditActionType::SYSTEM_CA_ROTATE)
                .await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SYSTEM_CA_ROTATE,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id));
        let details = row.details_json.expect("details");
        assert_eq!(details["triggered_by"], serde_json::json!("api"));
    }

    #[tokio::test]
    async fn rotate_ca_unmanaged_writes_denied_system_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (mut state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let mut snapshot = state.cert.ca_snapshot.borrow().clone();
        snapshot.managed = false;
        let (_tx, rx) = tokio::sync::watch::channel(snapshot);
        Arc::make_mut(&mut state).cert.ca_snapshot = rx;

        let response = rotate_ca(
            State(Arc::clone(&state)),
            CanManageGlobalSettings::new(AuthenticatedUser {
                user_id: uuid::Uuid::now_v7(),
                auth_method: AuthMethod::Password,
                permissions: vec![Permission::ManageGlobalSettings],
                jti: None,
                actor_display: None,
            }),
            None,
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        wait_for_system_audit_rows(&db, 1).await;
        let row =
            latest_system_audit_row(&db, uptrakit_audit_log::AuditActionType::SYSTEM_CA_ROTATE)
                .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("ca_rotation_not_available_for_unmanaged_ca")
        );
    }
}
