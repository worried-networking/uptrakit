//! HTTP handlers for access preset endpoints.
//!
//! Presets are code-defined bundles of roles (see [`AccessPreset`]). The list
//! endpoint returns all available presets; the apply endpoint replaces a user's
//! roles with the roles from a chosen preset.

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
use uptrakit_shared_types::AccessPreset;
use uuid::Uuid;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageUsers;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role, role_permission, user, user_role};

pub use uptrakit_web_api_types::access_presets::AccessPresetResponse;
pub use uptrakit_web_api_types::users::ApplyPresetRequest;

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
    CanManageUsers(_user): CanManageUsers,
    Path(user_id): Path<Uuid>,
    Json(body): Json<ApplyPresetRequest>,
) -> Response {
    use uptrakit_web_api_types::validation::Validate;

    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Parse the preset name.
    let preset: AccessPreset = match body.preset.parse() {
        Ok(p) => p,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unknown preset: {}", body.preset),
            );
        }
    };

    // Verify the user exists.
    let user_model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "User not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
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
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let current_role_ids: Vec<Uuid> = current_user_roles.iter().map(|ur| ur.role_id).collect();

    let current_has_manage =
        match roles_grant_manage_users_check(state.db(), &current_role_ids).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("DB error: {e}");
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

    if let Err(e) = UserRole::delete_many()
        .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&txn)
        .await
    {
        tracing::error!("Failed to delete user roles: {e}");
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
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit transaction: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Build response with updated roles/permissions.
    let permissions =
        crate::routes::auth::get_user_permissions(state.db(), state.default_tenant_id, user_id)
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
