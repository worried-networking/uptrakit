//! HTTP handlers for access preset endpoints.
//!
//! Presets are code-defined bundles of roles (see [`AccessPreset`]). The list
//! endpoint returns all available presets; the apply endpoint replaces a user's
//! roles with the roles from a chosen preset.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set, TransactionTrait,
};
use time::OffsetDateTime;
use uptrakit_shared_types::AccessPreset;
use uuid::Uuid;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageUsers;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role, role_permission, user, user_role};

pub use uptrakit_web_api_types::access_presets::AccessPresetResponse;
pub use uptrakit_web_api_types::users::ApplyPresetRequest;

fn emit_user_preset_audit(
    state: &AppState,
    caller: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    target_user_id: Uuid,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id, actor_display) =
        authenticated_user_audit_actor(caller, api_token_id);

    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::builder(uptrakit_audit_log::AuditActionType::USER_UPDATE)
            .tenant_scope(state.default_tenant_id)
            .actor(actor_type, actor_id)
            .actor_display_opt(actor_display)
            .target("user", target_user_id.to_string(), None)
            .outcome(outcome)
            .details(details)
            .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all available access presets
#[utoipa::path(
    get,
    path = "/api/v1/access-presets",
    responses(
        (status = 200, description = "List of access presets", body = Vec<AccessPresetResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_access_presets(CanManageUsers(_user): CanManageUsers) -> Response {
    let presets: Vec<AccessPresetResponse> = AccessPreset::all()
        .iter()
        .map(|p| AccessPresetResponse {
            name: p.as_str().to_string(),
            description: p.description().to_string(),
            roles: p.roles().iter().map(|r| (*r).to_string()).collect(),
        })
        .collect();

    (StatusCode::OK, Json(presets)).into_response()
}

/// Apply an access preset to a user
///
/// Replaces all of the user's role assignments with the roles defined by the
/// chosen preset. Includes lockout prevention: will reject the request if it
/// would remove the `manage_users` permission from the last holder.
#[utoipa::path(
    post,
    path = "/api/v1/users/{id}/apply-preset",
    params(
        ("id" = Uuid, Path, description = "User UUID")
    ),
    request_body = ApplyPresetRequest,
    responses(
        (status = 200, description = "Preset applied", body = uptrakit_web_api_types::users::UserWithRolesResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "User or preset not found"),
        (status = 409, description = "Would remove last manage_users holder")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn apply_preset(
    State(state): State<Arc<AppState>>,
    CanManageUsers(caller): CanManageUsers,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<ApplyPresetRequest>,
) -> Response {
    use uptrakit_web_api_types::validation::Validate;
    let api_token_id = api_token_id.map(|value| value.0);

    if let Err(e) = body.validate() {
        emit_user_preset_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "update_kind": "apply_preset",
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Parse the preset name.
    let preset: AccessPreset = match body.preset.parse() {
        Ok(p) => p,
        Err(_) => {
            emit_user_preset_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "update_kind": "apply_preset",
                    "preset": body.preset,
                    "reason_code": "unknown_access_preset",
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unknown preset: {}", body.preset),
            );
        }
    };

    // Verify the user exists.
    let user_model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            emit_user_preset_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "update_kind": "apply_preset",
                    "preset": preset.as_str(),
                    "reason_code": "user_not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "User not found");
        }
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_user_preset_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "update_kind": "apply_preset",
                    "preset": preset.as_str(),
                    "reason_code": "user_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Resolve preset role names to role IDs.
    let role_names = preset.roles();
    let role_models = match Role::find()
        .filter(
            role::Column::Name.is_in(role_names.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        )
        .all(state.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_user_preset_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "update_kind": "apply_preset",
                    "preset": preset.as_str(),
                    "reason_code": "preset_role_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if role_models.len() != role_names.len() {
        tracing::error!(
            "Preset '{}' references {} roles but only {} found in DB",
            preset,
            role_names.len(),
            role_models.len()
        );
        emit_user_preset_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::Failed,
            serde_json::json!({
                "update_kind": "apply_preset",
                "preset": preset.as_str(),
                "reason_code": "preset_role_configuration_mismatch",
            }),
        );
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Preset role configuration mismatch",
        );
    }

    let new_role_ids: Vec<Uuid> = role_models.iter().map(|r| r.id).collect();

    // Lockout prevention: check if this user currently has manage_users and the
    // new roles would remove it.
    let current_user_roles = match UserRole::find()
        .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .all(state.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_user_preset_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "update_kind": "apply_preset",
                    "preset": preset.as_str(),
                    "reason_code": "current_user_roles_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let current_role_ids: Vec<Uuid> = current_user_roles.iter().map(|ur| ur.role_id).collect();

    let current_has_manage =
        match roles_grant_manage_users_check(state.db(), &current_role_ids).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("DB error: {e}");
                emit_user_preset_audit(
                    &state,
                    &caller,
                    api_token_id,
                    user_id,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "update_kind": "apply_preset",
                        "preset": preset.as_str(),
                        "reason_code": "current_permission_check_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    if current_has_manage {
        let new_grants_manage = match roles_grant_manage_users_check(state.db(), &new_role_ids)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("DB error: {e}");
                emit_user_preset_audit(
                    &state,
                    &caller,
                    api_token_id,
                    user_id,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "update_kind": "apply_preset",
                        "preset": preset.as_str(),
                        "reason_code": "new_permission_check_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

        if !new_grants_manage {
            let other_count = match count_other_manage_users(
                state.db(),
                state.default_tenant_id,
                user_id,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("DB error: {e}");
                    emit_user_preset_audit(
                        &state,
                        &caller,
                        api_token_id,
                        user_id,
                        uptrakit_audit_log::AuditOutcome::Failed,
                        serde_json::json!({
                            "update_kind": "apply_preset",
                            "preset": preset.as_str(),
                            "reason_code": "manage_users_holder_count_failed",
                        }),
                    );
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            if other_count == 0 {
                emit_user_preset_audit(
                    &state,
                    &caller,
                    api_token_id,
                    user_id,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    serde_json::json!({
                        "update_kind": "apply_preset",
                        "preset": preset.as_str(),
                        "reason_code": "last_manage_users_holder",
                    }),
                );
                return error_response(
                    StatusCode::CONFLICT,
                    "Cannot remove manage_users permission from the last user who has it",
                );
            }
        }
    }

    // Replace roles in a transaction.
    let txn = match state.db().begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to start transaction: {e}");
            emit_user_preset_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "update_kind": "apply_preset",
                    "preset": preset.as_str(),
                    "reason_code": "preset_apply_transaction_start_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = UserRole::delete_many()
        .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&txn)
        .await
    {
        tracing::error!("Failed to delete user roles: {e}");
        emit_user_preset_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::Failed,
            serde_json::json!({
                "update_kind": "apply_preset",
                "preset": preset.as_str(),
                "reason_code": "existing_roles_delete_failed",
            }),
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    let now = OffsetDateTime::now_utc();
    for role_id in &new_role_ids {
        let new_ur = user_role::ActiveModel {
            tenant_id: Set(state.default_tenant_id),
            user_id: Set(user_id),
            role_id: Set(*role_id),
            assigned_at: Set(now),
        };
        if let Err(e) = new_ur.insert(&txn).await {
            tracing::error!("Failed to insert user role: {e}");
            emit_user_preset_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "update_kind": "apply_preset",
                    "preset": preset.as_str(),
                    "reason_code": "preset_role_insert_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit transaction: {e}");
        emit_user_preset_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::Failed,
            serde_json::json!({
                "update_kind": "apply_preset",
                "preset": preset.as_str(),
                "reason_code": "preset_apply_commit_failed",
            }),
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Build response with updated roles/permissions.
    let permissions = crate::middleware::require_auth::get_user_permissions(
        state.db(),
        state.default_tenant_id,
        user_id,
    )
    .await
    .unwrap_or_default();

    let role_summaries: Vec<uptrakit_web_api_types::users::UserRoleSummary> = role_models
        .iter()
        .map(|r| uptrakit_web_api_types::users::UserRoleSummary {
            id: r.id,
            name: r.name.clone(),
        })
        .collect();

    let response = uptrakit_web_api_types::users::UserWithRolesResponse {
        id: user_model.id,
        email: user_model.email.expose_email().to_string(),
        first_name: user_model.first_name.clone(),
        last_name: user_model.last_name.clone(),
        is_active: user_model.is_active,
        roles: role_summaries,
        permissions,
    };

    emit_user_preset_audit(
        &state,
        &caller,
        api_token_id,
        user_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "update_kind": "apply_preset",
            "preset": preset.as_str(),
            "role_names": role_names,
            "changed_fields": ["roles"],
        }),
    );

    (StatusCode::OK, Json(response)).into_response()
}

// ---------------------------------------------------------------------------
// Internal helpers (self-contained to avoid circular dependencies)
// ---------------------------------------------------------------------------

/// Check whether a set of role IDs grants the `manage_users` permission.
async fn roles_grant_manage_users_check(
    db: &sea_orm::DatabaseConnection,
    role_ids: &[Uuid],
) -> Result<bool, sea_orm::DbErr> {
    if role_ids.is_empty() {
        return Ok(false);
    }

    let perm = Permission::find()
        .filter(permission::Column::Name.eq("manage_users"))
        .one(db)
        .await?;

    let perm_id = match perm {
        Some(p) => p.id,
        None => return Ok(false),
    };

    let count = RolePermission::find()
        .filter(role_permission::Column::PermissionId.eq(perm_id))
        .filter(role_permission::Column::RoleId.is_in(role_ids.to_vec()))
        .count(db)
        .await?;

    Ok(count > 0)
}

/// Count other active users who hold `manage_users` in the given tenant.
async fn count_other_manage_users(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    exclude_user_id: Uuid,
) -> Result<u64, sea_orm::DbErr> {
    let perm = Permission::find()
        .filter(permission::Column::Name.eq("manage_users"))
        .one(db)
        .await?;

    let perm_id = match perm {
        Some(p) => p.id,
        None => return Ok(0),
    };

    let role_perms = RolePermission::find()
        .filter(role_permission::Column::PermissionId.eq(perm_id))
        .all(db)
        .await?;

    let role_ids: Vec<Uuid> = role_perms.iter().map(|rp| rp.role_id).collect();
    if role_ids.is_empty() {
        return Ok(0);
    }

    let user_roles = UserRole::find()
        .filter(user_role::Column::TenantId.eq(tenant_id))
        .filter(user_role::Column::RoleId.is_in(role_ids))
        .filter(user_role::Column::UserId.ne(exclude_user_id))
        .all(db)
        .await?;

    let other_user_ids: std::collections::HashSet<Uuid> =
        user_roles.iter().map(|ur| ur.user_id).collect();

    if other_user_ids.is_empty() {
        return Ok(0);
    }

    let count = User::find()
        .filter(user::Column::Id.is_in(other_user_ids.into_iter().collect::<Vec<_>>()))
        .filter(user::Column::IsActive.eq(true))
        .count(db)
        .await?;

    Ok(count)
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
        Set,
    };
    use uptrakit_shared_db::entity::audit_log;
    use uptrakit_shared_types::MaskedEmail;

    async fn latest_user_update_audit_row_for_target(
        db: &DatabaseConnection,
        target_user_id: Uuid,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::TenantId.is_not_null())
                .filter(
                    audit_log::Column::ActionType
                        .eq(uptrakit_audit_log::AuditActionType::USER_UPDATE),
                )
                .filter(audit_log::Column::TargetType.eq("user"))
                .filter(audit_log::Column::TargetId.eq(target_user_id.to_string()))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant user.update audit row");
    }

    async fn insert_target_user(db: &DatabaseConnection, email: &str) -> user::Model {
        let now = OffsetDateTime::now_utc();
        user::ActiveModel {
            id: Set(Uuid::now_v7()),
            email: Set(MaskedEmail::new(email.to_string())),
            first_name: Set("Preset".to_string()),
            last_name: Set("Target".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert target user")
    }

    #[tokio::test]
    async fn apply_preset_writes_user_update_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let target_user = insert_target_user(&app.db, "preset-target@test.local").await;

        let caller = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app.db)
            .await
            .expect("query caller user")
            .expect("caller user row");

        let status = client
            .post_json(
                &format!("/api/v1/users/{}/apply-preset", target_user.id),
                &ApplyPresetRequest {
                    preset: "operator".to_string(),
                },
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK);

        let row = latest_user_update_audit_row_for_target(&app.db, target_user.id).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(caller.id));
        let details = row.details_json.expect("details");
        assert_eq!(details["update_kind"], serde_json::json!("apply_preset"));
        assert_eq!(details["preset"], serde_json::json!("operator"));
        assert_eq!(details["changed_fields"], serde_json::json!(["roles"]));
    }

    #[tokio::test]
    async fn apply_unknown_preset_writes_validation_failed_user_update_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let target_user = insert_target_user(&app.db, "invalid-preset-target@test.local").await;

        let status = client
            .post_json(
                &format!("/api/v1/users/{}/apply-preset", target_user.id),
                &ApplyPresetRequest {
                    preset: "bogus".to_string(),
                },
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let row = latest_user_update_audit_row_for_target(&app.db, target_user.id).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(details["update_kind"], serde_json::json!("apply_preset"));
        assert_eq!(details["preset"], serde_json::json!("bogus"));
        assert_eq!(
            details["reason_code"],
            serde_json::json!("unknown_access_preset")
        );
    }
}
