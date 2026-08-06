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
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_shared_types::AccessPreset;
use uuid::Uuid;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::action::AccessAuthority;
use crate::middleware::permission::CanManageUsers;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use uptrakit_shared_db::access_grants::{
    GuardedMutation, LockoutVerdict, begin_guarded, check_lockout,
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
    let (actor_type, actor_id) = authenticated_user_audit_actor(caller, api_token_id);

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        uptrakit_audit_log::AuditActionType::USER_UPDATE,
    )
    .tenant_scope(state.default_tenant_id)
    .actor(actor_type, actor_id)
    .target("user", target_user_id.to_string(), None)
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_event(entry);
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
/// would remove the `manage_users` permission from the last holder, and
/// (M1.6a) the engine-backed `access:manage` lockout guard covers the same
/// role-replacement for the access:manage plane. Interim gap: this handler
/// still gates on the legacy `manage_users` permission via `CanManageUsers`
/// rather than the `users:manage`/`access:manage` split — a `users:manage`-only
/// caller can currently apply any preset, including ones that grant
/// `access:manage`; full conversion lands in M1.6b. Assigning a preset whose
/// roles reach the system plane additionally requires `system.access:manage`,
/// checked inline against the engine.
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
        (status = 409, description = "Would remove last manage_users or access:manage holder")
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
    Extension(authority): Extension<AccessAuthority>,
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
        .filter(role::Column::TenantId.is_null())
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

    // Replace roles in a transaction (IMMEDIATE — see begin_guarded's doc).
    let txn = match begin_guarded(state.db()).await {
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

    // System-plane fine check over the roles this preset actually ADDS
    // (shared with `update_user_roles`; see the helper's doc for the in-tx
    // re-read and fail-closed rationale).
    let reaches_system_plane = match crate::routes::access_grants::added_roles_reach_system_plane(
        &txn,
        state.default_tenant_id,
        user_id,
        &new_role_ids,
        "apply_preset",
    )
    .await
    {
        Ok(v) => v,
        Err(denied) => {
            drop(txn);
            return denied;
        }
    };
    if reaches_system_plane {
        // APPROVED: body-dependent fine check (corpus 07, restated invariant)
        if let Some(denied) =
            crate::middleware::action::require_system_access(&state.access_engine, &authority)
        {
            drop(txn);
            return denied;
        }
    }

    let verdict = match check_lockout(
        &txn,
        state.default_tenant_id,
        &GuardedMutation::SetUserRoles {
            tenant_id: state.default_tenant_id,
            user_id,
            new_role_ids: &new_role_ids,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("Failed to evaluate lockout guard: {e}");
            drop(txn);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    if !matches!(verdict, LockoutVerdict::Permitted) {
        drop(txn);
        return crate::routes::access_grants::lockout_denial_response(
            &state,
            uptrakit_audit_log::AuditActionType::USER_UPDATE.into(),
            authenticated_user_audit_actor(&caller, api_token_id),
            "user",
            user_id.to_string(),
            verdict,
        );
    }

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

    state.access_engine.invalidate_subjects(&[user_id], &[]);
    state
        .notification
        .notification_service
        .publish_controller_event(uptrakit_wire::ControllerMessage::AccessInvalidated(
            uptrakit_wire::AccessInvalidatedPayload::new(vec![user_id], vec![]),
        ))
        .await;

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
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::test_harness::TestApp;
    use crate::test_harness::fixtures;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
        Set,
    };
    use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
    use uptrakit_shared_db::entity::audit_log;
    use uptrakit_shared_types::MaskedEmail;
    use uptrakit_shared_types::access::Selector;

    async fn latest_audit_row_for_target(
        db: &DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
        target_user_id: Uuid,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::TenantId.is_not_null())
                .filter(audit_log::Column::ActionType.eq(action_type.as_str()))
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

        panic!("expected {action_type} audit row for user target");
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

        let row = latest_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_UPDATE,
            target_user.id,
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
        assert_eq!(row.actor_id, Some(caller.id));
        let details = row.details_json.expect("details");
        assert_eq!(details["update_kind"], serde_json::json!("apply_preset"));
        assert_eq!(details["preset"], serde_json::json!("operator"));
        assert_eq!(details["changed_fields"], serde_json::json!(["roles"]));
    }

    /// M16a-plan3 Task 2: a tenant-scoped role sharing a preset's role name
    /// must never shadow the global built-in — `apply_preset`'s
    /// `.is_in(role_names)` lookup must stay scoped to `tenant_id IS NULL`
    /// rows. Deterministic pre-fix: the unscoped query matches both the
    /// global "viewer" row and the shadow, so `role_models.len() (2) !=
    /// role_names.len() (1)` trips the existing mismatch guard and the
    /// endpoint returns 500 instead of silently assigning the shadow.
    #[tokio::test]
    async fn apply_preset_ignores_tenant_shadow() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let target_user = insert_target_user(&app.db, "preset-shadow-target@test.local").await;

        let global_viewer_id = Role::find()
            .filter(role::Column::Name.eq("viewer"))
            .filter(role::Column::TenantId.is_null())
            .one(&app.db)
            .await
            .expect("query")
            .expect("seeded global viewer role")
            .id;
        let shadow_viewer_id = fixtures::insert_shadow_role(&app.db, app.tenant_id, "viewer").await;

        let status = client
            .post_json(
                &format!("/api/v1/users/{}/apply-preset", target_user.id),
                &ApplyPresetRequest {
                    preset: "read_only".to_string(),
                },
            )
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "apply_preset must succeed once the by-name lookup is scoped to global rows"
        );

        let assigned_role_ids: Vec<Uuid> = user_role::Entity::find()
            .filter(user_role::Column::TenantId.eq(app.tenant_id))
            .filter(user_role::Column::UserId.eq(target_user.id))
            .all(&app.db)
            .await
            .expect("user_role query")
            .into_iter()
            .map(|r| r.role_id)
            .collect();
        assert_eq!(
            assigned_role_ids,
            vec![global_viewer_id],
            "apply_preset must assign only the global viewer role"
        );
        assert!(
            !assigned_role_ids.contains(&shadow_viewer_id),
            "the tenant shadow role must never be assigned by apply_preset"
        );
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

        let row = latest_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_UPDATE,
            target_user.id,
        )
        .await;
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

    #[tokio::test]
    async fn apply_preset_owner_preset_without_system_access_denied() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;
        let target_user = insert_target_user(&app.db, "owner-preset-target1@test.local").await;

        // A caller holding ONLY `settings_manager` has the legacy
        // `manage_users` permission (satisfying `CanManageUsers`) but no
        // system-plane authority — the owner test caller auto-holds
        // `system_administrator` too, so it cannot be used here.
        let (_caller_id, caller_token) = fixtures::register_user_with_only_role(
            &app,
            "tenant-admin-preset1@test.local",
            "settings_manager",
        )
        .await;

        let status = client
            .post_json(
                &format!("/api/v1/users/{}/apply-preset", target_user.id),
                &ApplyPresetRequest {
                    preset: "owner".to_string(),
                },
            )
            .bearer(&caller_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "applying the owner preset without system.access:manage must be denied"
        );
    }

    #[tokio::test]
    async fn apply_preset_tenant_plane_preset_without_system_access_still_allowed() {
        // Interim gap (owner decision 4, pinned not fixed): this handler
        // still gates on the legacy `manage_users` permission via
        // `CanManageUsers` rather than the `users:manage`/`access:manage`
        // split, so a principal lacking system-plane authority can still
        // apply any preset whose roles do NOT reach the system plane.
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;
        let target_user = insert_target_user(&app.db, "tenant-preset-target1@test.local").await;

        let (_caller_id, caller_token) = fixtures::register_user_with_only_role(
            &app,
            "tenant-admin-preset-readonly@test.local",
            "settings_manager",
        )
        .await;

        let status = client
            .post_json(
                &format!("/api/v1/users/{}/apply-preset", target_user.id),
                &ApplyPresetRequest {
                    preset: "read_only".to_string(),
                },
            )
            .bearer(&caller_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a tenant-plane preset must still be applyable without system.access:manage (interim gap)"
        );
    }

    #[tokio::test]
    async fn apply_preset_owner_preset_with_system_access_allowed() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;
        let target_user = insert_target_user(&app.db, "owner-preset-target2@test.local").await;

        // A caller holding ONLY `settings_manager` has the legacy
        // `manage_users` permission (satisfying `CanManageUsers`, this
        // endpoint's outer gate) and the engine `access:manage` grant, but
        // no system-plane authority -- mirroring
        // `apply_preset_owner_preset_without_system_access_denied` above.
        // Unlike the bootstrap owner (who auto-holds `system_administrator`
        // and would make the grant below redundant), this caller's pass
        // through the system-plane fine check depends entirely on the
        // explicit grant: deleting it must flip this test to FORBIDDEN.
        let (caller_id, caller_token) = fixtures::register_user_with_only_role(
            &app,
            "tenant-admin-preset2@test.local",
            "settings_manager",
        )
        .await;

        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::User(caller_id),
                tenant_id: None,
                patterns: &["system.access:manage".parse().expect("valid pattern")],
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("insert system-plane grant");
        app.state
            .access_engine
            .invalidate_subjects(&[caller_id], &[]);

        let status = client
            .post_json(
                &format!("/api/v1/users/{}/apply-preset", target_user.id),
                &ApplyPresetRequest {
                    preset: "owner".to_string(),
                },
            )
            .bearer(&caller_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "caller holding system.access:manage must be able to apply the owner preset"
        );
    }

    #[tokio::test]
    async fn apply_preset_lockout_denial_writes_user_update_denied_audit_event() {
        // The legacy `manage_users`-holder-count guard (lines above) and the
        // new engine-backed `SetUserRoles` guard are both role-driven, but
        // `settings_manager` is the ONLY built-in role granting both
        // `manage_users` (legacy) and `access:manage` (engine) — so any
        // built-in-role transition away from it trips the legacy guard
        // first, never reaching the new guard. To isolate the NEW guard,
        // the target's sole coverage of `access:manage` must come from a
        // role that carries no legacy `manage_users` permission: a
        // freshly-created custom role (no `role_permissions` rows),
        // mirroring `roles.rs`'s `deleting_last_covering_role_is_409_lockout`.
        let app = TestApp::new().await;
        let client = app.client();
        let owner_token = fixtures::open_registration(&app).await;
        let settings_manager_role_id = fixtures::role_id_by_name(&app, "settings_manager").await;

        let (status, created): (StatusCode, uptrakit_web_api_types::roles::RoleResponse) = client
            .post_json(
                "/api/v1/roles",
                &serde_json::json!({ "name": "sole-access-manage-role", "description": null }),
            )
            .bearer(&owner_token)
            .send_json()
            .await;
        assert_eq!(status, StatusCode::CREATED);

        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::Role(created.id),
                tenant_id: None,
                patterns: &["access:manage".parse().expect("valid pattern")],
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("stage covering role grant");

        let (holder_id, _holder_token) = fixtures::stage_zero_role_user(&app).await;
        fixtures::link_role(&app, holder_id, created.id).await;

        // Strip the owner's OWN `access:manage` coverage (via
        // `settings_manager`) so the new role's assignment is the sole
        // remaining covering holder tenant-wide. This leaves the owner's
        // legacy `manage_users` permission intact (only the engine grant
        // row is revoked), so the owner still passes `CanManageUsers`.
        fixtures::revoke_role_grants_covering(
            &app,
            settings_manager_role_id,
            &[uptrakit_shared_types::access::actions::ACCESS_MANAGE],
        )
        .await;

        let status = client
            .post_json(
                &format!("/api/v1/users/{holder_id}/apply-preset"),
                &ApplyPresetRequest {
                    preset: "read_only".to_string(),
                },
            )
            .bearer(&owner_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let row = latest_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_UPDATE,
            holder_id,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("lockout_access_manage")
        );
    }

    #[tokio::test]
    async fn apply_preset_reapplying_already_held_system_plane_roles_does_not_require_system_access()
     {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;

        let (caller_id, caller_token) = fixtures::register_user_with_only_role(
            &app,
            "sysplane-caller@test.local",
            "settings_manager",
        )
        .await;
        let sys_admin_role_id = fixtures::role_id_by_name(&app, "system_administrator").await;
        fixtures::link_role(&app, caller_id, sys_admin_role_id).await;

        let status = client
            .post_json(
                &format!("/api/v1/users/{caller_id}/apply-preset"),
                &ApplyPresetRequest {
                    preset: "owner".to_string(),
                },
            )
            .bearer(&caller_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "re-applying an already-held system-plane role must not require system.access:manage"
        );
    }
}
