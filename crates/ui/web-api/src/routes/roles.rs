//! HTTP handlers for role listing endpoints.
//!
//! All endpoints require the [`Permission::ManageUsers`] permission via the
//! [`CanManageUsers`] extractor. Roles are read-only (seeded by migrations).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::app_state::DbState;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageUsers;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role_permission};

pub use uptrakit_web_api_types::roles::RoleResponse;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a [`RoleResponse`] for a single role model by resolving its permissions.
async fn build_role_response(
    db: &DatabaseConnection,
    role_model: &uptrakit_shared_db::entity::role::Model,
) -> Result<RoleResponse, sea_orm::DbErr> {
    let role_perms = RolePermission::find()
        .filter(role_permission::Column::RoleId.eq(role_model.id))
        .all(db)
        .await?;

    let perm_ids: Vec<Uuid> = role_perms.iter().map(|rp| rp.permission_id).collect();

    let permissions = if perm_ids.is_empty() {
        vec![]
    } else {
        let perm_models = Permission::find()
            .filter(permission::Column::Id.is_in(perm_ids))
            .all(db)
            .await?;

        perm_models
            .into_iter()
            .filter_map(|p| p.name.parse::<uptrakit_shared_types::Permission>().ok())
            .collect()
    };

    Ok(RoleResponse {
        id: role_model.id,
        name: role_model.name.clone(),
        description: role_model.description.clone(),
        is_built_in: role_model.is_built_in,
        permissions,
    })
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all roles with their permissions
#[utoipa::path(
    get,
    path = "/api/v1/roles",
    responses(
        (status = 200, description = "List of roles with permissions", body = Vec<RoleResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_roles(
    State(db): State<DbState>,
    CanManageUsers(_user): CanManageUsers,
) -> Response {
    let roles = match Role::find().all(db.db()).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to list roles: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let mut responses = Vec::with_capacity(roles.len());
    for r in &roles {
        match build_role_response(db.db(), r).await {
            Ok(resp) => responses.push(resp),
            Err(e) => {
                tracing::error!("Failed to build role response for {}: {e}", r.id);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        }
    }

    (StatusCode::OK, Json(responses)).into_response()
}

/// Get a single role with its permissions
#[utoipa::path(
    get,
    path = "/api/v1/roles/{id}",
    params(
        ("id" = Uuid, Path, description = "Role UUID")
    ),
    responses(
        (status = 200, description = "Role details with permissions", body = RoleResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Role not found")
    ),
    tag = "Users",
    extensions(("x-required-permission" = json!("manage_users"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_role(
    State(db): State<DbState>,
    CanManageUsers(_user): CanManageUsers,
    Path(role_id): Path<Uuid>,
) -> Response {
    let role_model = match Role::find_by_id(role_id).one(db.db()).await {
        Ok(Some(r)) => r,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Role not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match build_role_response(db.db(), &role_model).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!("Failed to build role response: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
