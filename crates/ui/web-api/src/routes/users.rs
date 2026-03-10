//! HTTP handlers for user management endpoints.
//!
//! All endpoints require the [`Permission::ManageUsers`] permission via the
//! [`CanManageUsers`] extractor.

use std::sync::Arc;

use axum::{
    Json,
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
use crate::error_response::error_response;
use crate::middleware::permission::CanManageUsers;
use crate::routes::auth::get_user_permissions;
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
    CanManageUsers(_user): CanManageUsers,
    Path(user_id): Path<Uuid>,
    Json(body): Json<UpdateUserRolesRequest>,
) -> Response {
    use uptrakit_web_api_types::validation::Validate;

    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Verify the user exists.
    let user_model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
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
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if requested_roles.len() != body.role_ids.len() {
        return error_response(StatusCode::BAD_REQUEST, "One or more role IDs not found");
    }

    // Lockout prevention: if the user currently has manage_users and the new
    // roles would NOT grant it, check that other holders still exist.
    let current_has_manage = {
        let current_roles = match get_user_role_summaries(&state, user_id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("DB error: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        let current_role_ids: Vec<Uuid> = current_roles.iter().map(|r| r.id).collect();
        match roles_grant_manage_users(&state, &current_role_ids).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("DB error: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    };

    if current_has_manage {
        let new_grants_manage = match roles_grant_manage_users(&state, &body.role_ids).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("DB error: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

        if !new_grants_manage {
            let other_count = match count_other_manage_users_holders(&state, user_id).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("DB error: {e}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            };

            if other_count == 0 {
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
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit transaction: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    match build_user_response(&state, &user_model).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!("Failed to build user response: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
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
    Path(user_id): Path<Uuid>,
    Json(body): Json<UpdateUserActiveRequest>,
) -> Response {
    // Prevent self-deactivation.
    if !body.is_active && caller.user_id == user_id {
        return error_response(StatusCode::CONFLICT, "Cannot deactivate your own account");
    }

    let user_model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut active_model: user::ActiveModel = user_model.into();
    active_model.is_active = Set(body.is_active);
    active_model.deactivated_at = Set(if body.is_active { None } else { Some(now) });
    active_model.updated_at = Set(now);

    let updated = match active_model.update(state.db()).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Failed to update user: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match build_user_response(&state, &updated).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!("Failed to build user response: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
