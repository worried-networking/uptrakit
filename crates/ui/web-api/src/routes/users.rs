//! HTTP handlers for user management endpoints.
//!
//! All endpoints require the [`Permission::ManageUsers`] permission via the
//! [`CanManageUsers`] extractor.

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget cleanup sends on error paths intentionally drop results"
)]
#![expect(
    clippy::string_slice,
    reason = "slice index is at a validated char boundary"
)]

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
use uuid::Uuid;

use crate::AppState;
use crate::app_state::DbState;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageUsers;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
    get_user_permissions,
};
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role, role_permission, user, user_role};

pub use uptrakit_web_api_types::users::{
    ApplyPresetRequest, UpdateUserActiveRequest, UpdateUserRolesRequest, UserRoleSummary,
    UserWithRolesResponse,
};

/// Permission info for the listing endpoint.
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct PermissionInfo {
    pub name: String,
    pub description: String,
}

fn emit_user_update_audit(
    state: &AppState,
    caller: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    target_user_id: Uuid,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(caller, api_token_id);

    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::builder(uptrakit_audit_log::AuditActionType::USER_UPDATE)
            .tenant_scope(state.default_tenant_id)
            .actor(actor_type, actor_id)
            .target("user", target_user_id.to_string(), None)
            .outcome(outcome)
            .details(details)
            .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a [`UserWithRolesResponse`] for a single user model.
async fn build_user_response(
    state: &AppState,
    user_model: &user::Model,
) -> Result<UserWithRolesResponse, sea_orm::DbErr> {
    let roles = get_user_role_summaries(state, user_model.id).await?;
    let permissions = get_user_permissions(state.db(), state.default_tenant_id, user_model.id)
        .await
        .unwrap_or_default();

    Ok(UserWithRolesResponse {
        id: user_model.id,
        email: user_model.email.expose_email().to_string(),
        first_name: user_model.first_name.clone(),
        last_name: user_model.last_name.clone(),
        is_active: user_model.is_active,
        roles,
        permissions,
    })
}

/// Get role summaries for a user.
async fn get_user_role_summaries(
    state: &AppState,
    user_id: Uuid,
) -> Result<Vec<UserRoleSummary>, sea_orm::DbErr> {
    let user_roles = UserRole::find()
        .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .all(state.db())
        .await?;

    let role_ids: Vec<Uuid> = user_roles.iter().map(|ur| ur.role_id).collect();
    if role_ids.is_empty() {
        return Ok(vec![]);
    }

    let role_models = Role::find()
        .filter(role::Column::Id.is_in(role_ids.clone()))
        .all(state.db())
        .await?;

    Ok(role_models
        .into_iter()
        .map(|r| UserRoleSummary {
            id: r.id,
            name: r.name,
        })
        .collect())
}

/// Count how many *other* active users have the `manage_users` permission in
/// this tenant. Used for lockout prevention.
async fn count_other_manage_users_holders(
    state: &AppState,
    exclude_user_id: Uuid,
) -> Result<u64, sea_orm::DbErr> {
    // Find the permission row for manage_users.
    let perm = Permission::find()
        .filter(permission::Column::Name.eq("manage_users"))
        .one(state.db())
        .await?;

    let perm_id = match perm {
        Some(p) => p.id,
        None => return Ok(0),
    };

    // Get role IDs that grant manage_users.
    let role_perms = RolePermission::find()
        .filter(role_permission::Column::PermissionId.eq(perm_id))
        .all(state.db())
        .await?;

    let role_ids: Vec<Uuid> = role_perms.iter().map(|rp| rp.role_id).collect();
    if role_ids.is_empty() {
        return Ok(0);
    }

    // Count distinct users (other than the excluded one) who hold any of those
    // roles in the default tenant and are active.
    //
    // We do this in two steps to stay compatible with SQLite:
    // 1. Get user_ids from user_roles matching those role_ids + tenant.
    // 2. Count active users in that set, excluding the target user.
    let user_roles = UserRole::find()
        .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
        .filter(user_role::Column::RoleId.is_in(role_ids))
        .filter(user_role::Column::UserId.ne(exclude_user_id))
        .all(state.db())
        .await?;

    let other_user_ids: std::collections::HashSet<Uuid> =
        user_roles.iter().map(|ur| ur.user_id).collect();

    if other_user_ids.is_empty() {
        return Ok(0);
    }

    let count = User::find()
        .filter(user::Column::Id.is_in(other_user_ids.into_iter().collect::<Vec<_>>()))
        .filter(user::Column::IsActive.eq(true))
        .count(state.db())
        .await?;

    Ok(count)
}

/// Check whether a set of role IDs grants the `manage_users` permission.
async fn roles_grant_manage_users(
    state: &AppState,
    role_ids: &[Uuid],
) -> Result<bool, sea_orm::DbErr> {
    let perm = Permission::find()
        .filter(permission::Column::Name.eq("manage_users"))
        .one(state.db())
        .await?;

    let perm_id = match perm {
        Some(p) => p.id,
        None => return Ok(false),
    };

    let count = RolePermission::find()
        .filter(role_permission::Column::PermissionId.eq(perm_id))
        .filter(role_permission::Column::RoleId.is_in(role_ids.to_vec()))
        .count(state.db())
        .await?;

    Ok(count > 0)
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all users with their roles
#[utoipa::path(
    get,
    path = "/api/v1/users",
    responses(
        (status = 200, description = "List of users with roles", body = Vec<UserWithRolesResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_users(
    State(state): State<Arc<AppState>>,
    CanManageUsers(_user): CanManageUsers,
) -> Response {
    let users = match User::find().all(state.db()).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Failed to list users: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut responses = Vec::with_capacity(users.len());
    for u in &users {
        match build_user_response(&state, u).await {
            Ok(r) => responses.push(r),
            Err(e) => {
                tracing::error!("Failed to build user response for {}: {e}", u.id);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    }

    (StatusCode::OK, Json(responses)).into_response()
}

/// List all available permissions
#[utoipa::path(
    get,
    path = "/api/v1/permissions",
    responses(
        (status = 200, description = "List of all permissions", body = Vec<PermissionInfo>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_permissions(CanManageUsers(_user): CanManageUsers) -> Response {
    use crate::auth::permissions::Permission;

    let perms: Vec<PermissionInfo> = Permission::all()
        .into_iter()
        .map(|p| PermissionInfo {
            name: p.as_str().to_string(),
            description: p.description().to_string(),
        })
        .collect();

    (StatusCode::OK, Json(perms)).into_response()
}

/// Get a single user with roles and resolved permissions
#[utoipa::path(
    get,
    path = "/api/v1/users/{id}",
    params(
        ("id" = Uuid, Path, description = "User UUID")
    ),
    responses(
        (status = 200, description = "User details with roles and permissions", body = UserWithRolesResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "User not found")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_user(
    State(state): State<Arc<AppState>>,
    CanManageUsers(_user): CanManageUsers,
    Path(user_id): Path<Uuid>,
) -> Response {
    let user_model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match build_user_response(&state, &user_model).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!("Failed to build user response: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Replace a user's role assignments
#[utoipa::path(
    put,
    path = "/api/v1/users/{id}/roles",
    params(
        ("id" = Uuid, Path, description = "User UUID")
    ),
    request_body = UpdateUserRolesRequest,
    responses(
        (status = 200, description = "Roles updated", body = UserWithRolesResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "User not found"),
        (status = 409, description = "Would remove last manage_users holder")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_user_roles(
    State(state): State<Arc<AppState>>,
    CanManageUsers(caller): CanManageUsers,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<UpdateUserRolesRequest>,
) -> Response {
    use uptrakit_web_api_types::validation::Validate;
    let api_token_id = api_token_id.map(|value| value.0);

    if let Err(e) = body.validate() {
        emit_user_update_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "invalid_role_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Verify the user exists.
    let user_model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "user_not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "User not found");
        }
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "user_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify all requested role IDs exist.
    let requested_roles = match Role::find()
        .filter(role::Column::Id.is_in(body.role_ids.clone()))
        .all(state.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "role_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if requested_roles.len() != body.role_ids.len() {
        emit_user_update_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "reason_code": "role_not_found",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, "One or more role IDs not found");
    }

    let current_roles = match get_user_role_summaries(&state, user_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "current_roles_lookup_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let current_role_ids: Vec<Uuid> = current_roles.iter().map(|r| r.id).collect();

    // Lockout prevention: if the user currently has manage_users and the new
    // roles would NOT grant it, check that other holders still exist.
    let current_has_manage = match roles_grant_manage_users(&state, &current_role_ids).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "permission_check_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if current_has_manage {
        let new_grants_manage = match roles_grant_manage_users(&state, &body.role_ids).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("DB error: {e}");
                emit_user_update_audit(
                    &state,
                    &caller,
                    api_token_id,
                    user_id,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "reason_code": "new_permission_check_failed",
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

        if !new_grants_manage {
            let other_count = match count_other_manage_users_holders(&state, user_id).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("DB error: {e}");
                    emit_user_update_audit(
                        &state,
                        &caller,
                        api_token_id,
                        user_id,
                        uptrakit_audit_log::AuditOutcome::Failed,
                        serde_json::json!({
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
                emit_user_update_audit(
                    &state,
                    &caller,
                    api_token_id,
                    user_id,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    serde_json::json!({
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
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "user_update_transaction_start_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Delete existing role assignments.
    if let Err(e) = UserRole::delete_many()
        .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&txn)
        .await
    {
        tracing::error!("Failed to delete user roles: {e}");
        emit_user_update_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::Failed,
            serde_json::json!({
                "reason_code": "user_role_delete_failed",
            }),
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Insert new role assignments.
    let now = OffsetDateTime::now_utc();
    for role_id in &body.role_ids {
        let new_ur = user_role::ActiveModel {
            tenant_id: Set(state.default_tenant_id),
            user_id: Set(user_id),
            role_id: Set(*role_id),
            assigned_at: Set(now),
        };
        if let Err(e) = new_ur.insert(&txn).await {
            tracing::error!("Failed to insert user role: {e}");
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "user_role_insert_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit transaction: {e}");
        emit_user_update_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::Failed,
            serde_json::json!({
                "reason_code": "user_update_commit_failed",
            }),
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    let roles_changed = {
        let current: std::collections::BTreeSet<Uuid> = current_role_ids.iter().copied().collect();
        let requested: std::collections::BTreeSet<Uuid> = body.role_ids.iter().copied().collect();
        current != requested
    };
    let changed_fields = if roles_changed {
        vec!["roles"]
    } else {
        Vec::<&str>::new()
    };
    emit_user_update_audit(
        &state,
        &caller,
        api_token_id,
        user_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "changed_fields": changed_fields,
            "roles_changed": roles_changed,
        }),
    );

    match build_user_response(&state, &user_model).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!("Failed to build user response: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a user's profile (first_name / last_name)
#[utoipa::path(
    put,
    path = "/api/v1/users/{id}/profile",
    params(
        ("id" = Uuid, Path, description = "User UUID")
    ),
    request_body = uptrakit_web_api_types::profile::UpdateProfileRequest,
    responses(
        (status = 204, description = "Profile updated"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "User not found"),
        (status = 422, description = "Validation error")
    ),
    tag = "Users",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_profile(
    State(db): State<DbState>,
    Extension(auth_user): Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
    Json(req): Json<uptrakit_web_api_types::profile::UpdateProfileRequest>,
) -> Response {
    use crate::auth::permissions::Permission;
    use uptrakit_web_api_types::validation::Validate;

    if auth_user.user_id != user_id && !auth_user.permissions.contains(&Permission::ManageUsers) {
        return error_response(
            StatusCode::FORBIDDEN,
            "Cannot update another user's profile",
        );
    }

    if let Err(e) = req.validate() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{}: {}", e.field, e.message),
        );
    }

    let now = OffsetDateTime::now_utc();

    let model = match User::find_by_id(user_id).one(db.db()).await {
        Ok(Some(m)) => m,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to load user");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut active: user::ActiveModel = model.into();
    active.first_name = Set(req.first_name);
    active.last_name = Set(req.last_name);
    active.updated_at = Set(now);

    if let Err(e) = active.update(db.db()).await {
        tracing::error!(error = %e, "failed to update user profile");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    StatusCode::NO_CONTENT.into_response()
}

/// Activate or deactivate a user
#[utoipa::path(
    put,
    path = "/api/v1/users/{id}/active",
    params(
        ("id" = Uuid, Path, description = "User UUID")
    ),
    request_body = UpdateUserActiveRequest,
    responses(
        (status = 200, description = "User activation status updated", body = UserWithRolesResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "User not found"),
        (status = 409, description = "Cannot deactivate self")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_user_active(
    State(state): State<Arc<AppState>>,
    CanManageUsers(caller): CanManageUsers,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<UpdateUserActiveRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);

    // Prevent self-deactivation.
    if !body.is_active && caller.user_id == user_id {
        emit_user_update_audit(
            &state,
            &caller,
            api_token_id,
            user_id,
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "reason_code": "self_deactivation_blocked",
                "is_active": body.is_active,
            }),
        );
        return error_response(StatusCode::CONFLICT, "Cannot deactivate your own account");
    }

    let user_model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "user_not_found",
                    "is_active": body.is_active,
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "User not found");
        }
        Err(e) => {
            tracing::error!("DB error: {e}");
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "user_lookup_failed",
                    "is_active": body.is_active,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let previous_is_active = user_model.is_active;

    let now = OffsetDateTime::now_utc();
    let mut active_model: user::ActiveModel = user_model.into();
    active_model.is_active = Set(body.is_active);
    active_model.deactivated_at = Set(if body.is_active { None } else { Some(now) });
    active_model.updated_at = Set(now);

    let updated = match active_model.update(state.db()).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Failed to update user: {e}");
            emit_user_update_audit(
                &state,
                &caller,
                api_token_id,
                user_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "user_update_failed",
                    "is_active": body.is_active,
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let is_active_changed = previous_is_active != body.is_active;
    let changed_fields = if is_active_changed {
        vec!["is_active"]
    } else {
        Vec::<&str>::new()
    };
    emit_user_update_audit(
        &state,
        &caller,
        api_token_id,
        user_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "changed_fields": changed_fields,
            "is_active": body.is_active,
        }),
    );

    match build_user_response(&state, &updated).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!("Failed to build user response: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Initiate an email address change for a user (self-service, password-authenticated only).
///
/// Sends a confirmation email to the new address and a notification to the old address.
/// Returns 202 Accepted on success.
#[tracing::instrument(skip_all)]
pub async fn initiate_email_change(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
    external_base_url: Option<axum::Extension<crate::extract::ExternalBaseUrl>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<uptrakit_web_api_types::profile::InitiateEmailChangeRequest>,
) -> Response {
    use crate::auth::AuthMethod;
    use uptrakit_shared_db::entity::{email_change_request, prelude::*};
    use uptrakit_web_api_types::validation::Validate;

    // OIDC accounts cannot change email via this flow
    if !matches!(auth_user.auth_method, AuthMethod::Password) {
        return error_response(
            StatusCode::FORBIDDEN,
            "Email is managed by your identity provider",
        );
    }

    if auth_user.user_id != user_id {
        return error_response(StatusCode::FORBIDDEN, "Cannot change another user's email");
    }

    if let Err(e) = req.validate() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{}: {}", e.field, e.message),
        );
    }

    let user = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to load user");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Verify current password (constant-time path for missing hash)
    let hash = match &user.password_hash {
        Some(h) => h.expose_secret().to_string(),
        None => {
            let _ = crate::auth::password::verify_password("dummy", "$argon2id$dummy");
            return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
        }
    };
    match crate::auth::password::verify_password(req.current_password.expose_secret(), &hash) {
        Ok(true) => {}
        Ok(false) => {
            return error_response(StatusCode::UNAUTHORIZED, "Current password is incorrect");
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to verify password");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Check new email is not already taken
    if let Ok(Some(_)) = User::find()
        .filter(
            user::Column::Email.eq(uptrakit_shared_types::MaskedEmail::new(
                req.new_email.as_str(),
            )),
        )
        .one(state.db())
        .await
    {
        return error_response(StatusCode::CONFLICT, "Email address is already in use");
    }

    // Generate confirmation token
    let raw_token = match crate::auth::token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to generate secure token");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let token_hash = crate::auth::token::hash_token(&raw_token);

    let now = OffsetDateTime::now_utc();
    let expires_at = now + time::Duration::hours(24);

    let encrypted_email = match uptrakit_crypto::EncryptedString::new(
        req.new_email.clone(),
        "uptrakit:email_change_requests:new_email",
    ) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "failed to encrypt new email");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Derive base URL from middleware or headers
    let base_url = external_base_url
        .map(|axum::Extension(u)| u.0)
        .or_else(|| {
            headers
                .get("origin")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim_end_matches('/').to_string())
        })
        .or_else(|| {
            headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .map(|h| format!("https://{h}"))
        });
    let base_url = match base_url {
        Some(url) => url,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot determine base URL",
            );
        }
    };
    let confirm_url = format!("{}/auth/email-change/confirm?token={}", base_url, raw_token);

    // Send both emails BEFORE saving — failure returns 503
    match send_email_change_emails(
        &state,
        state.default_tenant_id,
        req.new_email.as_str(),
        user.email.expose_email(),
        &confirm_url,
    )
    .await
    {
        Ok(()) => {}
        Err(e) => return error_response(StatusCode::SERVICE_UNAVAILABLE, e),
    }

    // Save pending request (delete-then-insert in a transaction)
    let txn = match state.db().begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "failed to begin transaction");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let _ = EmailChangeRequest::delete_many()
        .filter(email_change_request::Column::UserId.eq(user_id))
        .exec(&txn)
        .await;

    let record = email_change_request::ActiveModel {
        id: Set(Uuid::now_v7()),
        user_id: Set(user_id),
        new_email: Set(encrypted_email),
        token_hash: Set(token_hash),
        expires_at: Set(expires_at),
        created_at: Set(now),
    };

    if let Err(e) = record.insert(&txn).await {
        tracing::error!(error = %e, "failed to insert email change request");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = txn.commit().await {
        tracing::error!(error = %e, "failed to commit email change request");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    StatusCode::ACCEPTED.into_response()
}

/// Change a user's password (self-service, password-authenticated only).
///
/// `PUT /api/v1/users/{id}/password`
#[tracing::instrument(skip_all)]
pub async fn change_password(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(req): Json<uptrakit_web_api_types::profile::ChangePasswordRequest>,
) -> Response {
    use crate::auth::AuthMethod;
    use uptrakit_shared_db::entity::session;
    use uptrakit_web_api_types::validation::Validate;

    // OIDC accounts cannot change password
    if !matches!(auth_user.auth_method, AuthMethod::Password) {
        return error_response(
            StatusCode::FORBIDDEN,
            "Password change is not available for OIDC accounts",
        );
    }

    if auth_user.user_id != user_id {
        return error_response(
            StatusCode::FORBIDDEN,
            "Cannot change another user's password",
        );
    }

    if let Err(e) = req.validate() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{}: {}", e.field, e.message),
        );
    }

    let user = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!(error = %e, "failed to load user");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Constant-time path for missing password hash
    let hash = match &user.password_hash {
        Some(h) => h.expose_secret().to_string(),
        None => {
            let _ = crate::auth::password::verify_password("dummy", "$argon2id$dummy");
            return error_response(StatusCode::UNAUTHORIZED, "Invalid credentials");
        }
    };

    match crate::auth::password::verify_password(req.current_password.expose_secret(), &hash) {
        Ok(true) => {}
        Ok(false) => {
            return error_response(StatusCode::UNAUTHORIZED, "Current password is incorrect");
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to verify password");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    let new_hash = match crate::auth::password::hash_password(req.new_password.expose_secret()) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "failed to hash new password");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut active: user::ActiveModel = user.into();
    active.password_hash = Set(Some(new_hash));
    active.updated_at = Set(now);

    if let Err(e) = active.update(state.db()).await {
        tracing::error!(error = %e, "failed to update password");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Preserve the current session (identified by refresh token cookie); revoke all others.
    let session_service = crate::auth::session::SessionService::new(state.db().clone());
    let refresh_token_opt = extract_refresh_token_from_headers(&headers);

    if let Some(refresh_token) = refresh_token_opt {
        let token_hash = crate::auth::token::hash_token(&refresh_token);
        let current_session = Session::find()
            .filter(session::Column::RefreshTokenHash.eq(token_hash))
            .one(state.db())
            .await
            .ok()
            .flatten();

        if let Some(current_session) = current_session {
            let _ = session_service
                .delete_user_sessions_except(user_id, current_session.id)
                .await;
        } else {
            let _ = session_service.delete_user_sessions(user_id).await;
        }
    } else {
        let _ = session_service.delete_user_sessions(user_id).await;
    }

    // Deny all other access tokens; keep the current JTI alive.
    let now_ts = now.unix_timestamp();
    let expiry_secs = crate::auth::jwt::ACCESS_TOKEN_EXPIRY_SECS;
    if let Some(jti) = &auth_user.jti {
        state
            .auth
            .token_denylist
            .deny_user_except(
                user_id,
                jti,
                now_ts + expiry_secs,
                now_ts,
                now_ts + expiry_secs,
            )
            .await;
    } else {
        state
            .auth
            .token_denylist
            .deny_user(user_id, now_ts, now_ts + expiry_secs)
            .await;
    }

    // Propagate token revocation to other controller instances (best-effort).
    state
        .notification
        .notification_service
        .publish_controller_event(uptrakit_wire::ControllerMessage::TokenRevoked(
            uptrakit_wire::TokenRevokedPayload {
                jti: None,
                exp: None,
                user_id: Some(user_id),
                iat_cutoff: Some(now_ts),
                purge_after: Some(now_ts + expiry_secs),
            },
        ))
        .await;

    emit_user_update_audit(
        &state,
        &auth_user,
        None,
        user_id,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({ "changed_fields": ["password"] }),
    );

    StatusCode::NO_CONTENT.into_response()
}

fn extract_refresh_token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("refresh_token=") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Cancel a pending email change for a user (self-service only).
///
/// `DELETE /api/v1/users/{id}/email`
#[tracing::instrument(skip_all)]
pub async fn cancel_email_change(
    State(db): State<DbState>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
) -> Response {
    use uptrakit_shared_db::entity::{email_change_request, prelude::*};

    if auth_user.user_id != user_id {
        return error_response(
            StatusCode::FORBIDDEN,
            "Cannot cancel another user's email change",
        );
    }

    let result = EmailChangeRequest::delete_many()
        .filter(email_change_request::Column::UserId.eq(user_id))
        .exec(db.db())
        .await;

    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to cancel email change");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

async fn send_email_change_emails(
    state: &AppState,
    tenant_id: Uuid,
    new_email: &str,
    old_email: &str,
    confirm_url: &str,
) -> Result<(), String> {
    use uptrakit_plugin_infrastructure_registry::{TransactionalEmailError, escape_html};

    let tenant_db = uptrakit_web_api_queries::TenantDb::new(state.db().clone(), tenant_id);

    // Message 1: to new address — confirm link
    let body1 = format!(
        "A request was made to change the email on account {old_email}. \
        Confirm your new address by clicking the link below (expires in 24 hours).\n\n\
        {confirm_url}\n\nIf you did not request this, contact your administrator.",
    );
    let body1_html = format!(
        "<p>A request was made to change the email on account \
        <strong>{old_email_esc}</strong>.</p>\
        <p>Confirm your new address by clicking the link below (expires in 24 hours).</p>\
        <p><a href=\"{url}\">{url}</a></p>\
        <p>If you did not request this, contact your administrator.</p>",
        old_email_esc = escape_html(old_email),
        url = escape_html(confirm_url),
    );

    state
        .plugin_ops
        .send_transactional_email(
            &tenant_db,
            new_email,
            "Confirm your new email address \u{2014} Uptrakit",
            &body1,
            &body1_html,
        )
        .await
        .map_err(|e| match e {
            TransactionalEmailError::NotConfigured => "Email delivery not configured".to_string(),
            TransactionalEmailError::DeliveryFailed(_) => "Email delivery failed".to_string(),
            _ => {
                tracing::warn!(
                    ?e,
                    "unhandled TransactionalEmailError variant sending confirmation"
                );
                "Email delivery failed".to_string()
            }
        })?;

    // Message 2: to old address — notification
    let masked_new = mask_email(new_email);
    let body2 = format!(
        "A request was made to change the email address on account {old_email} \
        to {masked_new}. To cancel this change, sign in and go to Profile \u{2192} \
        Cancel pending change.",
    );
    let body2_html = format!(
        "<p>A request was made to change the email address on account \
        <strong>{old_email_esc}</strong> to <strong>{masked_new_esc}</strong>.</p>\
        <p>To cancel this change, sign in and go to Profile \u{2192} Cancel pending change.</p>",
        old_email_esc = escape_html(old_email),
        masked_new_esc = escape_html(&masked_new),
    );

    state
        .plugin_ops
        .send_transactional_email(
            &tenant_db,
            old_email,
            "Email address change requested \u{2014} Uptrakit",
            &body2,
            &body2_html,
        )
        .await
        .map_err(|e| match e {
            TransactionalEmailError::NotConfigured => "Email delivery not configured".to_string(),
            TransactionalEmailError::DeliveryFailed(_) => "Email delivery failed".to_string(),
            _ => {
                tracing::warn!(
                    ?e,
                    "unhandled TransactionalEmailError variant sending notification"
                );
                "Email delivery failed".to_string()
            }
        })?;

    Ok(())
}

fn mask_email(email: &str) -> String {
    if let Some(at_pos) = email.find('@') {
        let local = &email[..at_pos];
        let domain = &email[at_pos..];
        if local.is_empty() {
            return email.to_string();
        }
        format!("{}***{}", &local[..1], domain)
    } else {
        email.to_string()
    }
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
            first_name: Set("Target".to_string()),
            last_name: Set("User".to_string()),
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
    async fn update_user_roles_writes_user_update_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let target_user = insert_target_user(&app.db, "roles-target@test.local").await;

        let caller = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app.db)
            .await
            .expect("query caller user")
            .expect("caller user row");

        let viewer_role = Role::find()
            .filter(role::Column::Name.eq("viewer"))
            .one(&app.db)
            .await
            .expect("query viewer role")
            .expect("viewer role");

        let req = UpdateUserRolesRequest {
            role_ids: vec![viewer_role.id],
        };
        let status = client
            .put_json(&format!("/api/v1/users/{}/roles", target_user.id), &req)
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK);

        let row = latest_user_update_audit_row_for_target(&app.db, target_user.id).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::USER_UPDATE,
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
        assert_eq!(row.actor_id, Some(caller.id));
        assert_eq!(row.target_type.as_deref(), Some("user"));
        let expected_target_id = target_user.id.to_string();
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));

        let details = row.details_json.expect("details");
        assert_eq!(details["roles_changed"], serde_json::json!(true));
        assert_eq!(details["changed_fields"], serde_json::json!(["roles"]));
    }

    #[tokio::test]
    async fn update_user_active_writes_user_update_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;
        let target_user = insert_target_user(&app.db, "active-target@test.local").await;

        let caller = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app.db)
            .await
            .expect("query caller user")
            .expect("caller user row");

        let req = UpdateUserActiveRequest { is_active: false };
        let status = client
            .put_json(&format!("/api/v1/users/{}/active", target_user.id), &req)
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK);

        let row = latest_user_update_audit_row_for_target(&app.db, target_user.id).await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::USER_UPDATE,
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
        assert_eq!(row.actor_id, Some(caller.id));
        assert_eq!(row.target_type.as_deref(), Some("user"));
        let expected_target_id = target_user.id.to_string();
        assert_eq!(row.target_id.as_deref(), Some(expected_target_id.as_str()));

        let details = row.details_json.expect("details");
        assert_eq!(details["is_active"], serde_json::json!(false));
        assert_eq!(details["changed_fields"], serde_json::json!(["is_active"]));
    }

    #[tokio::test]
    async fn self_deactivation_writes_denied_user_update_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let caller = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app.db)
            .await
            .expect("query caller user")
            .expect("caller user row");

        let req = UpdateUserActiveRequest { is_active: false };
        let status = client
            .put_json(&format!("/api/v1/users/{}/active", caller.id), &req)
            .bearer(&access_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let row = latest_user_update_audit_row_for_target(&app.db, caller.id).await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("self_deactivation_blocked")
        );
    }
}
