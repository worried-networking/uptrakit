//! HTTP handlers for user management endpoints.
//!
//! Lifecycle reads/writes (list, get, activate/deactivate, profile) gate on
//! `users:manage` via [`CanManageUsers`]; role assignment
//! ([`update_user_roles`]) gates on `access:manage` via `CanManageAccess` —
//! assigning a role whose grants reach the system plane additionally
//! requires `system.access:manage`, checked inline against the engine.

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
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqliteTransactionMode,
    TransactionOptions, TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::AppState;
use crate::app_state::{AccessState, DbState};
use crate::error_response::error_response;
use crate::extract::Unvalidated;
use crate::middleware::action::{
    AccessAuthority, CanManageAccess, CanManageUsers, record_access_deny, require_system_access,
};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
    get_user_permissions,
};
use uptrakit_audit_log::{AuditEntry, AuditOutcome, Event, Stateful};
use uptrakit_shared_db::access_grants::{GuardedMutation, LockoutVerdict, check_lockout};
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{role, user, user_role};
use uptrakit_shared_types::access::{Decision, actions};
use uptrakit_web_api_queries::queries::users::{UserView, update_user_active_in_tx};

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
    security(("oauth2" = ["users:manage"]), ("developer_token" = []))
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
///
/// Legacy permission catalog — removed in M1.8 (the action catalog endpoint
/// replaces it in M1.6b).
#[utoipa::path(
    get,
    path = "/api/v1/permissions",
    responses(
        (status = 200, description = "List of all permissions", body = Vec<PermissionInfo>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Users",
    security(("oauth2" = ["users:manage"]), ("developer_token" = []))
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
    security(("oauth2" = ["users:manage"]), ("developer_token" = []))
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

/// Audit snapshot for a user's role-assignment set.
///
/// Distinct from `UserView`: role assignment is a separate transition from
/// the user's own lifecycle fields, so it gets its own before/after shape and
/// `user_role.update` action rather than reusing `user.update`.
#[derive(uptrakit_audit_log::AuditView)]
#[audit(target_type = "user", id_field = "user_id")]
struct UserRolesView {
    user_id: Uuid,
    role_ids: Vec<Uuid>,
}

/// Replace a user's role assignments
///
/// Adding a role whose grants reach the system plane additionally requires
/// system.access:manage (evaluated at runtime).
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
        (status = 409, description = "Would remove the last remaining access administrator")
    ),
    tag = "Users",
    security(("oauth2" = ["access:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_user_roles(
    State(state): State<Arc<AppState>>,
    CanManageAccess(caller): CanManageAccess,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Extension(authority): Extension<AccessAuthority>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<UpdateUserRolesRequest>,
) -> Response {
    use uptrakit_audit_log::AuditActionType;
    use uptrakit_web_api_types::validation::Validate;
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);

    if let Err(e) = body.validate() {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
            .tenant_scope(state.default_tenant_id)
            .actor(actor_type, actor_id)
            .target("user", user_id.to_string(), None)
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({ "reason_code": "invalid_role_request" }))
            .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Verify the user exists.
    let user_model = match User::find_by_id(user_id).one(state.db()).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                .tenant_scope(state.default_tenant_id)
                .actor(actor_type, actor_id)
                .target("user", user_id.to_string(), None)
                .outcome(AuditOutcome::Denied)
                .details(serde_json::json!({ "reason_code": "user_not_found" }))
                .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "User not found");
        }
        Err(e) => {
            tracing::error!("DB error: {e}");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                .tenant_scope(state.default_tenant_id)
                .actor(actor_type, actor_id)
                .target("user", user_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({ "reason_code": "user_lookup_failed" }))
                .build()
            {
                state.audit_emitter.emit_event(entry);
            }
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
            if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                .tenant_scope(state.default_tenant_id)
                .actor(actor_type, actor_id)
                .target("user", user_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({ "reason_code": "role_lookup_failed" }))
                .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if requested_roles.len() != body.role_ids.len() {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
            .tenant_scope(state.default_tenant_id)
            .actor(actor_type, actor_id)
            .target("user", user_id.to_string(), None)
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({ "reason_code": "role_not_found" }))
            .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::BAD_REQUEST, "One or more role IDs not found");
    }

    let current_roles = match get_user_role_summaries(&state, user_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error: {e}");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                .tenant_scope(state.default_tenant_id)
                .actor(actor_type, actor_id)
                .target("user", user_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({ "reason_code": "current_roles_lookup_failed" }))
                .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let current_role_ids: Vec<Uuid> = current_roles.iter().map(|r| r.id).collect();

    // Replace roles in a transaction (IMMEDIATE to avoid SQLITE_BUSY_SNAPSHOT).
    let txn = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to start transaction: {e}");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                .tenant_scope(state.default_tenant_id)
                .actor(actor_type, actor_id)
                .target("user", user_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({
                    "reason_code": "user_update_transaction_start_failed",
                }))
                .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // System-plane fine check over the roles this request actually ADDS
    // (shared with `apply_preset`; see the helper's doc for the in-tx
    // re-read and fail-closed rationale).
    let reaches_system_plane = match crate::routes::access_grants::added_roles_reach_system_plane(
        &txn,
        state.default_tenant_id,
        user_id,
        &body.role_ids,
        "update_user_roles",
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
        if let Some(denied) = require_system_access(&state.access_engine, &authority) {
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
            new_role_ids: &body.role_ids,
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
            AuditActionType::USER_ROLE_UPDATE.into(),
            (actor_type, actor_id),
            "user",
            user_id.to_string(),
            verdict,
        );
    }

    // Delete existing role assignments.
    if let Err(e) = UserRole::delete_many()
        .filter(user_role::Column::TenantId.eq(state.default_tenant_id))
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&txn)
        .await
    {
        tracing::error!("Failed to delete user roles: {e}");
        if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
            .tenant_scope(state.default_tenant_id)
            .actor(actor_type, actor_id)
            .target("user", user_id.to_string(), None)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "user_role_delete_failed" }))
            .build()
        {
            state.audit_emitter.emit_event(entry);
        }
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
            if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                .tenant_scope(state.default_tenant_id)
                .actor(actor_type, actor_id)
                .target("user", user_id.to_string(), None)
                .outcome(AuditOutcome::Failed)
                .details(serde_json::json!({ "reason_code": "user_role_insert_failed" }))
                .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
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

    let before_view = UserRolesView {
        user_id,
        role_ids: current_role_ids.clone(),
    };
    let after_view = UserRolesView {
        user_id,
        role_ids: body.role_ids.clone(),
    };
    let hook = state.audit_emitter.commit_hook();
    if let Ok(audit_entry) = AuditEntry::<Stateful>::user_role_update(&before_view, &after_view)
        .tenant_scope(state.default_tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "changed_fields": changed_fields,
            "roles_changed": roles_changed,
            "new_role_ids": body.role_ids,
        }))
        .build()
    {
        let _ = state
            .audit_emitter
            .emit_stateful(&txn, &hook, audit_entry)
            .await;
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit transaction: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    state.access_engine.invalidate_subjects(&[user_id], &[]);
    state
        .notification
        .notification_service
        .publish_controller_event(uptrakit_wire::ControllerMessage::AccessInvalidated(
            uptrakit_wire::AccessInvalidatedPayload::new(vec![user_id], vec![]),
        ))
        .await;

    match build_user_response(&state, &user_model).await {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!("Failed to build user response: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a user's profile (first_name / last_name)
///
/// Self-service (any authenticated user may update their own profile);
/// updating another user's profile additionally requires `users:manage`.
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
    security(("oauth2" = []), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_profile(
    State(db): State<DbState>,
    State(access): State<AccessState>,
    Extension(authority): Extension<AccessAuthority>,
    Extension(auth_user): Extension<AuthenticatedUser>,
    Path(user_id): Path<Uuid>,
    Json(req): Json<uptrakit_web_api_types::profile::UpdateProfileRequest>,
) -> Response {
    use uptrakit_web_api_types::validation::Validate;

    if auth_user.user_id != user_id {
        let Some(ctx) = authority.ready() else {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Authorization authority unavailable",
            );
        };
        // APPROVED: body-dependent fine check (corpus 07, restated invariant)
        match access.0.authorize(ctx, &actions::USERS_MANAGE, None) {
            Decision::Allow => {}
            Decision::Deny(reason) => {
                record_access_deny(&reason);
                return error_response(
                    StatusCode::FORBIDDEN,
                    "Cannot update another user's profile",
                );
            }
            // `Decision` is #[non_exhaustive] in another crate: unknown
            // variants deny, fail-closed.
            _ => {
                return error_response(
                    StatusCode::FORBIDDEN,
                    "Cannot update another user's profile",
                );
            }
        }
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
        (status = 400, description = "Invalid request body"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "User not found"),
        (status = 409, description = "Cannot deactivate self")
    ),
    tag = "Users",
    security(("oauth2" = ["users:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_user_active(
    State(state): State<Arc<AppState>>,
    CanManageUsers(caller): CanManageUsers,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(user_id): Path<Uuid>,
    body: Unvalidated<UpdateUserActiveRequest>,
) -> Response {
    use uptrakit_audit_log::AuditActionType;
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&caller, api_token_id);

    let body = match body.require_valid() {
        Ok(body) => body,
        Err(e) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                .tenant_scope(state.default_tenant_id)
                .actor(actor_type, actor_id)
                .target("user", user_id.to_string(), None)
                .outcome(AuditOutcome::ValidationFailed)
                .details(serde_json::json!({ "reason_code": "invalid_request" }))
                .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::BAD_REQUEST, e.to_string());
        }
    };

    // Prevent self-deactivation.
    if !body.is_active && caller.user_id == user_id {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
            .tenant_scope(state.default_tenant_id)
            .actor(actor_type, actor_id)
            .target("user", user_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({
                "reason_code": "self_deactivation_blocked",
                "is_active": body.is_active,
            }))
            .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::CONFLICT, "Cannot deactivate your own account");
    }

    // Open a BEGIN IMMEDIATE transaction: we read-then-write the user row.
    let txn = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to start transaction: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if !body.is_active {
        let verdict = match check_lockout(
            &txn,
            state.default_tenant_id,
            &GuardedMutation::DeactivateUser { user_id },
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
                AuditActionType::USER_UPDATE.into(),
                (actor_type, actor_id),
                "user",
                user_id.to_string(),
                verdict,
            );
        }
    }

    let (before_model, after_model) =
        match update_user_active_in_tx(&txn, user_id, body.is_active).await {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                drop(txn);
                if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                    .tenant_scope(state.default_tenant_id)
                    .actor(actor_type, actor_id)
                    .target("user", user_id.to_string(), None)
                    .outcome(AuditOutcome::Denied)
                    .details(serde_json::json!({
                        "reason_code": "user_not_found",
                        "is_active": body.is_active,
                    }))
                    .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::NOT_FOUND, "User not found");
            }
            Err(e) => {
                tracing::error!("Failed to update user: {e}");
                drop(txn);
                if let Ok(entry) = AuditEntry::<Event>::builder_event(AuditActionType::USER_UPDATE)
                    .tenant_scope(state.default_tenant_id)
                    .actor(actor_type, actor_id)
                    .target("user", user_id.to_string(), None)
                    .outcome(AuditOutcome::Failed)
                    .details(serde_json::json!({
                        "reason_code": "user_update_failed",
                        "is_active": body.is_active,
                    }))
                    .build()
                {
                    state.audit_emitter.emit_event(entry);
                }
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let is_active_changed = before_model.is_active != body.is_active;
    let changed_fields = if is_active_changed {
        vec!["is_active"]
    } else {
        Vec::<&str>::new()
    };

    let before_view = UserView::from(&before_model);
    let after_view = UserView::from(&after_model);
    let hook = state.audit_emitter.commit_hook();
    if let Ok(audit_entry) = AuditEntry::<Stateful>::user_update(&before_view, &after_view)
        .tenant_scope(state.default_tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "changed_fields": changed_fields,
            "is_active": body.is_active,
        }))
        .build()
    {
        let _ = state
            .audit_emitter
            .emit_stateful(&txn, &hook, audit_entry)
            .await;
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit user active update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // Invalidate on both directions: deactivation shrinks live authority,
    // and activation must also flush any warmed negative cache entries from
    // while the account was inactive — one code path for both.
    if is_active_changed {
        state.access_engine.invalidate_subjects(&[user_id], &[]);
        state
            .notification
            .notification_service
            .publish_controller_event(uptrakit_wire::ControllerMessage::AccessInvalidated(
                uptrakit_wire::AccessInvalidatedPayload::new(vec![user_id], vec![]),
            ))
            .await;
    }

    match build_user_response(&state, &after_model).await {
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
#[utoipa::path(
    post,
    path = "/api/v1/users/{id}/email",
    params(
        ("id" = uuid::Uuid, Path, description = "User UUID")
    ),
    request_body = uptrakit_web_api_types::profile::InitiateEmailChangeRequest,
    responses(
        (status = 202, description = "Email change initiated; confirmation email sent"),
        (status = 401, description = "Not authenticated or wrong current password"),
        (status = 403, description = "Not authorized or account uses OIDC"),
        (status = 404, description = "User not found"),
        (status = 409, description = "Email already in use"),
        (status = 422, description = "Validation error"),
        (status = 503, description = "Email delivery unavailable")
    ),
    tag = "Users",
    security(("bearer_token" = []))
)]
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
#[utoipa::path(
    put,
    path = "/api/v1/users/{id}/password",
    params(
        ("id" = uuid::Uuid, Path, description = "User UUID")
    ),
    request_body = uptrakit_web_api_types::profile::ChangePasswordRequest,
    responses(
        (status = 204, description = "Password changed; other sessions invalidated"),
        (status = 401, description = "Not authenticated or wrong current password"),
        (status = 403, description = "Not authorized or account uses OIDC"),
        (status = 404, description = "User not found"),
        (status = 422, description = "Validation error")
    ),
    tag = "Users",
    security(("bearer_token" = []))
)]
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

    let (pw_actor_type, pw_actor_id) = authenticated_user_audit_actor(&auth_user, None);
    if let Ok(entry) =
        AuditEntry::<Event>::builder_event(uptrakit_audit_log::AuditActionType::USER_UPDATE)
            .tenant_scope(state.default_tenant_id)
            .actor(pw_actor_type, pw_actor_id)
            .target("user", user_id.to_string(), None)
            .outcome(AuditOutcome::Success)
            .details(serde_json::json!({ "changed_fields": ["password"] }))
            .build()
    {
        state.audit_emitter.emit_event(entry);
    }

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
#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}/email",
    params(
        ("id" = uuid::Uuid, Path, description = "User UUID")
    ),
    responses(
        (status = 204, description = "Pending email change cancelled (no-op if none exists)"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Users",
    security(("bearer_token" = []))
)]
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
        .plugin
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
        .plugin
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
    use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, insert_grant};
    use uptrakit_shared_db::entity::audit_log;
    use uptrakit_shared_types::access::Selector;
    use uptrakit_shared_types::{MaskedEmail, SecretString};

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
    async fn update_user_roles_writes_user_role_update_audit_event() {
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

        let row = latest_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_ROLE_UPDATE,
            target_user.id,
        )
        .await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::USER_ROLE_UPDATE,
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

        let row = latest_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_UPDATE,
            target_user.id,
        )
        .await;
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

        let row = latest_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_UPDATE,
            caller.id,
        )
        .await;
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

    #[tokio::test]
    async fn update_user_active_password_hash_absent_from_audit_snapshots() {
        let app = TestApp::new().await;
        let client = app.client();
        let access_token = fixtures::register_and_get_token(&client).await;

        let target_user = {
            let now = OffsetDateTime::now_utc();
            user::ActiveModel {
                id: Set(Uuid::now_v7()),
                email: Set(MaskedEmail::new("hash-check@test.local".to_string())),
                first_name: Set("Hash".to_string()),
                last_name: Set("Check".to_string()),
                password_hash: Set(Some(SecretString::new("$argon2id$known-secret-hash-value"))),
                is_active: Set(true),
                deactivated_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&app.db)
            .await
            .expect("insert target user with hash")
        };

        let status = client
            .put_json(
                &format!("/api/v1/users/{}/active", target_user.id),
                &UpdateUserActiveRequest { is_active: false },
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
        let known_hash = "$argon2id$known-secret-hash-value";

        let before = row.before_snapshot.expect("before_snapshot");
        let after = row.after_snapshot.expect("after_snapshot");

        assert!(
            before.get("password_hash").is_none(),
            "password_hash key must not appear in before_snapshot"
        );
        assert!(
            after.get("password_hash").is_none(),
            "password_hash key must not appear in after_snapshot"
        );
        assert!(
            !before.to_string().contains(known_hash),
            "known password hash value must not appear in before_snapshot JSON"
        );
        assert!(
            !after.to_string().contains(known_hash),
            "known password hash value must not appear in after_snapshot JSON"
        );
    }

    #[tokio::test]
    async fn update_user_roles_adding_system_plane_role_without_system_access_denied() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;
        let target_user = insert_target_user(&app.db, "sysplane-target1@test.local").await;

        let (_caller_id, caller_token) = fixtures::stage_user_with_grant(
            &app,
            "tenant-admin1@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;

        let sys_admin_role = Role::find()
            .filter(role::Column::Name.eq("system_administrator"))
            .one(&app.db)
            .await
            .expect("query system_administrator role")
            .expect("system_administrator role row");

        let req = UpdateUserRolesRequest {
            role_ids: vec![sys_admin_role.id],
        };
        let status = client
            .put_json(&format!("/api/v1/users/{}/roles", target_user.id), &req)
            .bearer(&caller_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "assigning a system-plane role without system.access:manage must be denied"
        );
    }

    #[tokio::test]
    async fn update_user_roles_adding_system_plane_role_with_system_access_allowed() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;
        let target_user = insert_target_user(&app.db, "sysplane-target2@test.local").await;

        let (caller_id, caller_token) = fixtures::stage_user_with_grant(
            &app,
            "tenant-admin2@test.local",
            &["access:manage"],
            Some(app.tenant_id),
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

        let sys_admin_role = Role::find()
            .filter(role::Column::Name.eq("system_administrator"))
            .one(&app.db)
            .await
            .expect("query system_administrator role")
            .expect("system_administrator role row");

        let req = UpdateUserRolesRequest {
            role_ids: vec![sys_admin_role.id],
        };
        let status = client
            .put_json(&format!("/api/v1/users/{}/roles", target_user.id), &req)
            .bearer(&caller_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "caller holding system.access:manage must be able to assign a system-plane role"
        );
    }

    #[tokio::test]
    async fn update_user_roles_lockout_denial_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;

        let owner = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app.db)
            .await
            .expect("query owner user")
            .expect("owner user row");
        let settings_manager_role_id = fixtures::role_id_by_name(&app, "settings_manager").await;
        UserRole::delete_many()
            .filter(user_role::Column::TenantId.eq(app.tenant_id))
            .filter(user_role::Column::UserId.eq(owner.id))
            .filter(user_role::Column::RoleId.eq(settings_manager_role_id))
            .exec(&app.db)
            .await
            .expect("strip owner's settings_manager assignment");
        app.state
            .access_engine
            .invalidate_subjects(&[owner.id], &[]);

        let (sole_holder_id, sole_holder_token) =
            fixtures::stage_user_with_only_role(&app, "settings_manager").await;
        let viewer_role_id = fixtures::role_id_by_name(&app, "viewer").await;

        let req = UpdateUserRolesRequest {
            role_ids: vec![viewer_role_id],
        };
        let status = client
            .put_json(&format!("/api/v1/users/{}/roles", sole_holder_id), &req)
            .bearer(&sole_holder_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let row = latest_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_ROLE_UPDATE,
            sole_holder_id,
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
    async fn update_user_active_deactivation_lockout_denial_writes_denied_audit_event() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;

        let owner = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app.db)
            .await
            .expect("query owner user")
            .expect("owner user row");
        let settings_manager_role_id = fixtures::role_id_by_name(&app, "settings_manager").await;
        UserRole::delete_many()
            .filter(user_role::Column::TenantId.eq(app.tenant_id))
            .filter(user_role::Column::UserId.eq(owner.id))
            .filter(user_role::Column::RoleId.eq(settings_manager_role_id))
            .exec(&app.db)
            .await
            .expect("strip owner's settings_manager assignment");
        app.state
            .access_engine
            .invalidate_subjects(&[owner.id], &[]);

        let (sole_holder_id, _sole_holder_token) =
            fixtures::stage_user_with_only_role(&app, "settings_manager").await;

        let (_caller_id, caller_token) = fixtures::stage_user_with_grant(
            &app,
            "active-lockout-caller@test.local",
            &["users:manage"],
            Some(app.tenant_id),
        )
        .await;

        let req = UpdateUserActiveRequest { is_active: false };
        let status = client
            .put_json(&format!("/api/v1/users/{}/active", sole_holder_id), &req)
            .bearer(&caller_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let row = latest_audit_row_for_target(
            &app.db,
            uptrakit_audit_log::AuditActionType::USER_UPDATE,
            sole_holder_id,
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
    async fn update_profile_self_service_allowed_but_other_user_requires_users_manage_denied() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;

        let (self_user_id, self_token) = fixtures::stage_zero_role_user(&app).await;

        // Self-service update succeeds without any users:manage grant.
        let self_status = client
            .put_json(
                &format!("/api/v1/users/{self_user_id}/profile"),
                &uptrakit_web_api_types::profile::UpdateProfileRequest {
                    first_name: "Self".to_string(),
                    last_name: "Updated".to_string(),
                },
            )
            .bearer(&self_token)
            .send_status()
            .await;
        assert_eq!(self_status, StatusCode::NO_CONTENT);

        // Updating another user's profile without users:manage is denied.
        let owner = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app.db)
            .await
            .expect("query owner user")
            .expect("owner user row");
        let other_status = client
            .put_json(
                &format!("/api/v1/users/{}/profile", owner.id),
                &uptrakit_web_api_types::profile::UpdateProfileRequest {
                    first_name: "Hijacked".to_string(),
                    last_name: "Name".to_string(),
                },
            )
            .bearer(&self_token)
            .send_status()
            .await;
        assert_eq!(other_status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn split_users_manage_can_lifecycle_but_not_assign_and_vice_versa() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;
        let viewer_role_id = fixtures::role_id_by_name(&app, "viewer").await;

        // users:manage-only: lifecycle changes allowed, role assignment denied.
        let (_lifecycle_admin_id, lifecycle_admin_token) = fixtures::stage_user_with_grant(
            &app,
            "split-lifecycle-admin@test.local",
            &["users:manage"],
            Some(app.tenant_id),
        )
        .await;
        let lifecycle_target =
            insert_target_user(&app.db, "split-target-lifecycle@test.local").await;

        let status = client
            .get("/api/v1/users")
            .bearer(&lifecycle_admin_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK, "users:manage must permit listing");

        let status = client
            .put_json(
                &format!("/api/v1/users/{}/active", lifecycle_target.id),
                &UpdateUserActiveRequest { is_active: false },
            )
            .bearer(&lifecycle_admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "users:manage must permit lifecycle changes"
        );

        let status = client
            .put_json(
                &format!("/api/v1/users/{}/roles", lifecycle_target.id),
                &UpdateUserRolesRequest {
                    role_ids: vec![viewer_role_id],
                },
            )
            .bearer(&lifecycle_admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "users:manage alone must not permit role assignment"
        );

        // access:manage-only: role assignment allowed, lifecycle denied.
        let (_access_admin_id, access_admin_token) = fixtures::stage_user_with_grant(
            &app,
            "split-access-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        let access_target = insert_target_user(&app.db, "split-target-access@test.local").await;

        let status = client
            .get("/api/v1/users")
            .bearer(&access_admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "access:manage alone must not permit listing"
        );

        let status = client
            .put_json(
                &format!("/api/v1/users/{}/active", access_target.id),
                &UpdateUserActiveRequest { is_active: false },
            )
            .bearer(&access_admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "access:manage alone must not permit lifecycle changes"
        );

        let status = client
            .put_json(
                &format!("/api/v1/users/{}/roles", access_target.id),
                &UpdateUserRolesRequest {
                    role_ids: vec![viewer_role_id],
                },
            )
            .bearer(&access_admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "access:manage must permit role assignment"
        );
    }

    #[tokio::test]
    async fn deactivating_last_access_manage_holder_is_409_and_swap_is_legal() {
        // -- Leg 1: deactivation. The self-deactivation guard fires first
        // for a sole holder deactivating themselves (covered by
        // `self_deactivation_writes_denied_user_update_audit_event`), so
        // this exercises a DIFFERENT caller deactivating the sole holder:
        // denied while sole, then legal once a second holder exists.
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;

        let owner = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app.db)
            .await
            .expect("query owner user")
            .expect("owner user row");
        let settings_manager_role_id = fixtures::role_id_by_name(&app, "settings_manager").await;
        UserRole::delete_many()
            .filter(user_role::Column::TenantId.eq(app.tenant_id))
            .filter(user_role::Column::UserId.eq(owner.id))
            .filter(user_role::Column::RoleId.eq(settings_manager_role_id))
            .exec(&app.db)
            .await
            .expect("strip owner's settings_manager assignment");
        app.state
            .access_engine
            .invalidate_subjects(&[owner.id], &[]);

        let (holder_a_id, _holder_a_token) =
            fixtures::stage_user_with_only_role(&app, "settings_manager").await;
        let (caller_b_id, caller_b_token) = fixtures::stage_user_with_grant(
            &app,
            "swap-deactivate-caller@test.local",
            &["users:manage"],
            Some(app.tenant_id),
        )
        .await;

        let status = client
            .put_json(
                &format!("/api/v1/users/{holder_a_id}/active"),
                &UpdateUserActiveRequest { is_active: false },
            )
            .bearer(&caller_b_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "deactivating the sole access:manage holder must be blocked"
        );
        let still_active = User::find_by_id(holder_a_id)
            .one(&app.db)
            .await
            .expect("query holder A")
            .expect("holder A row");
        assert!(still_active.is_active, "holder A must still be active");

        // Grant caller B coverage too — now two holders, so B deactivating
        // A is legal (A drops, B remains).
        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::User(caller_b_id),
                tenant_id: Some(app.tenant_id),
                patterns: &["access:manage".parse().expect("valid pattern")],
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("grant caller B access:manage");
        app.state
            .access_engine
            .invalidate_subjects(&[caller_b_id], &[]);

        let status = client
            .put_json(
                &format!("/api/v1/users/{holder_a_id}/active"),
                &UpdateUserActiveRequest { is_active: false },
            )
            .bearer(&caller_b_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "deactivating A is legal once B also covers access:manage"
        );
        let now_inactive = User::find_by_id(holder_a_id)
            .one(&app.db)
            .await
            .expect("query holder A")
            .expect("holder A row");
        assert!(!now_inactive.is_active, "holder A must now be inactive");

        // -- Leg 2: role swap, in an isolated app so leg 1's second holder
        // (caller B) cannot mask the lockout guard here. A sole holder's
        // covering role may be swapped for another covering role (legal,
        // post-state still covers), but swapping to a non-covering role
        // set is not — the brief's "PUT roles [] -> 409" is unreachable
        // literally (`UpdateUserRolesRequest::validate()` rejects an empty
        // `role_ids` with 400 before the handler runs), so a non-empty,
        // non-covering replacement (`viewer`) stands in for it here.
        let app2 = TestApp::new().await;
        let client2 = app2.client();
        let owner_token2 = fixtures::open_registration(&app2).await;

        let (status, created_role): (StatusCode, uptrakit_web_api_types::roles::RoleResponse) =
            client2
                .post_json(
                    "/api/v1/roles",
                    &uptrakit_web_api_types::roles::CreateRoleRequest {
                        name: "swap-role-b".to_string(),
                        description: None,
                    },
                )
                .bearer(&owner_token2)
                .send_json()
                .await;
        assert_eq!(status, StatusCode::CREATED);
        insert_grant(
            &app2.db,
            NewGrant {
                subject: GrantSubject::Role(created_role.id),
                tenant_id: None,
                patterns: &["access:manage".parse().expect("valid pattern")],
                selector: Selector::All,
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("grant role B access:manage coverage");

        let owner2 = User::find()
            .filter(user::Column::Email.eq("owner@test.local"))
            .one(&app2.db)
            .await
            .expect("query owner user")
            .expect("owner user row");
        let settings_manager_role_id2 = fixtures::role_id_by_name(&app2, "settings_manager").await;
        UserRole::delete_many()
            .filter(user_role::Column::TenantId.eq(app2.tenant_id))
            .filter(user_role::Column::UserId.eq(owner2.id))
            .filter(user_role::Column::RoleId.eq(settings_manager_role_id2))
            .exec(&app2.db)
            .await
            .expect("strip owner's settings_manager assignment");
        app2.state
            .access_engine
            .invalidate_subjects(&[owner2.id], &[]);

        let (holder_e_id, holder_e_token) = fixtures::stage_zero_role_user(&app2).await;
        fixtures::link_role(&app2, holder_e_id, settings_manager_role_id2).await;
        let viewer_role_id2 = fixtures::role_id_by_name(&app2, "viewer").await;

        let status = client2
            .put_json(
                &format!("/api/v1/users/{holder_e_id}/roles"),
                &UpdateUserRolesRequest {
                    role_ids: vec![created_role.id],
                },
            )
            .bearer(&holder_e_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "swapping covering role A for covering role B must be legal"
        );

        let status = client2
            .put_json(
                &format!("/api/v1/users/{holder_e_id}/roles"),
                &UpdateUserRolesRequest {
                    role_ids: vec![viewer_role_id2],
                },
            )
            .bearer(&holder_e_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "swapping the last covering role away must be blocked"
        );
    }

    #[tokio::test]
    async fn adding_authority_never_trips_lockout_409() {
        // `check_lockout` only guards `SetUserRoles`/`DeactivateUser`;
        // role/grant creation and user activation never invoke it at all
        // (adding-only mutations, per its doc comment). `SetUserRoles` IS
        // always evaluated (even for a pure add), but a post-state check
        // can only fail when authority shrinks, so exercising all four ops
        // here demonstrates none of them can ever produce a lockout 409.
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;

        let (_admin_id, admin_token) = fixtures::stage_user_with_grant(
            &app,
            "adding-authority-admin@test.local",
            &["access:manage", "users:manage"],
            Some(app.tenant_id),
        )
        .await;

        // role create — unguarded.
        let (status, created_role): (StatusCode, uptrakit_web_api_types::roles::RoleResponse) =
            client
                .post_json(
                    "/api/v1/roles",
                    &uptrakit_web_api_types::roles::CreateRoleRequest {
                        name: "adding-authority-role".to_string(),
                        description: None,
                    },
                )
                .bearer(&admin_token)
                .send_json()
                .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "role create must never be lockout-denied"
        );

        // grant create — unguarded.
        let (status, _created_grant): (
            StatusCode,
            uptrakit_web_api_types::access_grants::AccessGrantResponse,
        ) = client
            .post_json(
                "/api/v1/access/grants",
                &uptrakit_web_api_types::access_grants::CreateAccessGrantRequest {
                    subject_type:
                        uptrakit_web_api_types::access_grants::GrantSubjectTypeParam::Role,
                    subject_id: created_role.id,
                    patterns: vec!["hosts:read".to_string()],
                    selector: Selector::All,
                    description: None,
                },
            )
            .bearer(&admin_token)
            .send_json()
            .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "grant create must never be lockout-denied"
        );

        // assignment-add — a zero-role user gains a role.
        let (target_id, _target_token) = fixtures::stage_zero_role_user(&app).await;
        let status = client
            .put_json(
                &format!("/api/v1/users/{target_id}/roles"),
                &UpdateUserRolesRequest {
                    role_ids: vec![created_role.id],
                },
            )
            .bearer(&admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "role assignment-add must never be lockout-denied"
        );

        // activate — unguarded (only deactivation invokes check_lockout).
        let now = OffsetDateTime::now_utc();
        let inactive_target = user::ActiveModel {
            id: Set(Uuid::now_v7()),
            email: Set(MaskedEmail::new(
                "adding-authority-inactive@test.local".to_string(),
            )),
            first_name: Set("Inactive".to_string()),
            last_name: Set("Target".to_string()),
            password_hash: Set(None),
            is_active: Set(false),
            deactivated_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&app.db)
        .await
        .expect("insert inactive target user");
        let status = client
            .put_json(
                &format!("/api/v1/users/{}/active", inactive_target.id),
                &UpdateUserActiveRequest { is_active: true },
            )
            .bearer(&admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "activation must never be lockout-denied"
        );
    }

    #[tokio::test]
    async fn role_assignment_takes_effect_without_relogin() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;

        let (zero_id, zero_token) = fixtures::stage_zero_role_user(&app).await;
        let status = client
            .get("/api/v1/hosts")
            .bearer(&zero_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "zero-role user must be denied before assignment"
        );

        let (_admin_id, admin_token) = fixtures::stage_user_with_grant(
            &app,
            "relogin-admin@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        let viewer_role_id = fixtures::role_id_by_name(&app, "viewer").await;
        let status = client
            .put_json(
                &format!("/api/v1/users/{zero_id}/roles"),
                &UpdateUserRolesRequest {
                    role_ids: vec![viewer_role_id],
                },
            )
            .bearer(&admin_token)
            .send_status()
            .await;
        assert_eq!(status, StatusCode::OK);

        // SAME token, no re-login: the engine cache must be flushed by the
        // endpoint's own invalidation, not by re-minting a JWT.
        let status = client
            .get("/api/v1/hosts")
            .bearer(&zero_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "role assignment must take effect without re-login"
        );
    }

    #[tokio::test]
    async fn assigning_system_administrator_requires_system_access_manage() {
        let app = TestApp::new().await;
        let client = app.client();
        fixtures::open_registration(&app).await;

        let sys_admin_role = Role::find()
            .filter(role::Column::Name.eq("system_administrator"))
            .one(&app.db)
            .await
            .expect("query system_administrator role")
            .expect("system_administrator role row");

        // access:manage-only: denied.
        let target_denied = insert_target_user(&app.db, "sysadmin-assign-denied@test.local").await;
        let (_denied_admin_id, denied_admin_token) = fixtures::stage_user_with_grant(
            &app,
            "sysadmin-assign-tenant-only@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        let status = client
            .put_json(
                &format!("/api/v1/users/{}/roles", target_denied.id),
                &UpdateUserRolesRequest {
                    role_ids: vec![sys_admin_role.id],
                },
            )
            .bearer(&denied_admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "access:manage alone must not permit assigning system_administrator"
        );

        // access:manage + system.access:manage: allowed.
        let target_allowed =
            insert_target_user(&app.db, "sysadmin-assign-allowed@test.local").await;
        let (allowed_admin_id, allowed_admin_token) = fixtures::stage_user_with_grant(
            &app,
            "sysadmin-assign-system@test.local",
            &["access:manage"],
            Some(app.tenant_id),
        )
        .await;
        insert_grant(
            &app.db,
            NewGrant {
                subject: GrantSubject::User(allowed_admin_id),
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
            .invalidate_subjects(&[allowed_admin_id], &[]);

        let status = client
            .put_json(
                &format!("/api/v1/users/{}/roles", target_allowed.id),
                &UpdateUserRolesRequest {
                    role_ids: vec![sys_admin_role.id],
                },
            )
            .bearer(&allowed_admin_token)
            .send_status()
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "system.access:manage must permit assigning system_administrator"
        );
    }
}
