use crate::AppState;
use crate::auth::permissions::Permission;
use crate::auth::{AuthError, password, session::SessionService, token::generate_uuid};
use crate::middleware::require_auth::AuthenticatedUser;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{permission, role, role_permission, user, user_role};

pub use uptrakit_web_api_types::auth::{
    AuthResponse, LoginRequest, LogoutRequest, RefreshRequest, RefreshResponse, RegisterRequest,
    UserResponse,
};

/// Register a new user
#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "User registered successfully", body = AuthResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "Email already exists")
    ),
    tag = "Authentication"
)]
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Response {
    // Check if password auth is enabled
    if !state.settings.authentication().await.password_auth_enabled {
        return (StatusCode::FORBIDDEN, "Password authentication is disabled").into_response();
    }

    // Validate password length
    if req.password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            "Password must be at least 8 characters",
        )
            .into_response();
    }

    // Validate registration is allowed
    if let Err((status, msg)) = state
        .settings
        .registration()
        .await
        .validate(req.registration_token.as_deref())
    {
        return (status, msg).into_response();
    }

    // Hash password
    let password_hash = match password::hash_password(&req.password) {
        Ok(hash) => hash,
        Err(e) => {
            tracing::error!("Password hashing failed: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Check if user already exists
    let existing = User::find()
        .filter(user::Column::Email.eq(&req.email))
        .one(&state.db)
        .await;

    if let Ok(Some(_)) = existing {
        return (StatusCode::CONFLICT, "Email already exists").into_response();
    }

    // Check if this is the first user
    let user_count = match User::find().count(&state.db).await {
        Ok(count) => count,
        Err(e) => {
            tracing::error!(error = %e, "failed to count users");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let is_first_user = user_count == 0;

    // Create user
    let user_id = generate_uuid();
    let now = OffsetDateTime::now_utc();

    let new_user = user::ActiveModel {
        id: Set(user_id),
        email: Set(req.email.clone()),
        first_name: Set(req.first_name.clone()),
        last_name: Set(req.last_name.clone()),
        password_hash: Set(Some(password_hash)),
        is_active: Set(true),
        deactivated_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    if let Err(e) = new_user.insert(&state.db).await {
        tracing::error!("Failed to create user: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // If first user, assign admin role and complete initial setup
    if is_first_user {
        if let Err(e) = assign_admin_role(&state.db, user_id).await {
            tracing::error!("Failed to assign admin role to first user: {:?}", e);
            // Continue anyway - user is created
        }
        if let Err(e) = state
            .settings
            .registration_write()
            .await
            .complete_initial_setup(&state.db)
            .await
        {
            tracing::error!("Failed to complete initial registration setup: {:?}", e);
        }
    } else {
        // Non-first users get the 'user' role
        if let Err(e) = assign_user_role(&state.db, user_id).await {
            tracing::error!("Failed to assign user role: {:?}", e);
        }
    }

    // Get user permissions
    let permissions = match get_user_permissions(&state.db, user_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to get user permissions: {:?}", e);
            vec![]
        }
    };

    // Create refresh token
    let session_service = SessionService::new(state.db.clone());
    let refresh_token = match session_service
        .create_refresh_token(user_id, AuthMethod::Password, None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create refresh token: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Create JWT access token
    let access_token = match state
        .jwt
        .create_access_token(user_id, &permissions, "password", None)
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create access token: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response = AuthResponse {
        access_token,
        refresh_token,
        expires_in: state.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: UserResponse {
            id: user_id.to_string(),
            email: req.email,
            first_name: req.first_name,
            last_name: req.last_name,
            permissions,
        },
    };

    (StatusCode::CREATED, Json(response)).into_response()
}

/// Login with email and password
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 401, description = "Invalid credentials"),
        (status = 403, description = "User is deactivated")
    ),
    tag = "Authentication"
)]
pub async fn login(State(state): State<Arc<AppState>>, Json(req): Json<LoginRequest>) -> Response {
    // Check if password auth is enabled
    if !state.settings.authentication().await.password_auth_enabled {
        return (StatusCode::FORBIDDEN, "Password authentication is disabled").into_response();
    }

    // Find user by email
    let user = match User::find()
        .filter(user::Column::Email.eq(&req.email))
        .one(&state.db)
        .await
    {
        Ok(Some(user)) => user,
        _ => {
            return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
        }
    };

    // Check if user is active
    if !user.is_active {
        return (StatusCode::FORBIDDEN, "User is deactivated").into_response();
    }

    // Verify password
    let hash = match user.password_hash.as_ref() {
        Some(h) => h,
        None => {
            return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
        }
    };

    let valid = match password::verify_password(&req.password, hash) {
        Ok(valid) => valid,
        Err(e) => {
            tracing::error!("Password verification error: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !valid {
        return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
    }

    // Get user permissions
    let permissions = match get_user_permissions(&state.db, user.id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to get user permissions: {:?}", e);
            vec![]
        }
    };

    // Create refresh token
    let session_service = SessionService::new(state.db.clone());
    let refresh_token = match session_service
        .create_refresh_token(user.id, AuthMethod::Password, None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create refresh token: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Create JWT access token
    let access_token = match state
        .jwt
        .create_access_token(user.id, &permissions, "password", None)
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create access token: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response = AuthResponse {
        access_token,
        refresh_token,
        expires_in: state.jwt.expires_in(),
        token_type: "Bearer".to_string(),
        user: UserResponse {
            id: user.id.to_string(),
            email: user.email,
            first_name: user.first_name,
            last_name: user.last_name,
            permissions,
        },
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Logout and revoke refresh token
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    request_body = LogoutRequest,
    responses(
        (status = 204, description = "Logout successful"),
    ),
    tag = "Authentication"
)]
pub async fn logout(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LogoutRequest>,
) -> Response {
    // Revoke refresh token
    let session_service = SessionService::new(state.db.clone());
    if let Err(e) = session_service
        .revoke_refresh_token(&req.refresh_token)
        .await
    {
        tracing::error!("Failed to revoke refresh token: {:?}", e);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// Get current user information
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current user information", body = UserResponse),
        (status = 401, description = "Not authenticated")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
pub async fn me(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
) -> Response {
    // Get user info from DB (fresh data)
    let user = match User::find_by_id(auth_user.user_id).one(&state.db).await {
        Ok(Some(user)) => user,
        _ => {
            return (StatusCode::UNAUTHORIZED, "User not found").into_response();
        }
    };

    if !user.is_active {
        return (StatusCode::FORBIDDEN, "User is deactivated").into_response();
    }

    // Get fresh user permissions from DB
    let permissions = match get_user_permissions(&state.db, user.id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to get user permissions: {:?}", e);
            vec![]
        }
    };

    let response = UserResponse {
        id: user.id.to_string(),
        email: user.email,
        first_name: user.first_name,
        last_name: user.last_name,
        permissions,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Refresh an access token using a refresh token
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Token refreshed", body = RefreshResponse),
        (status = 401, description = "Invalid or expired refresh token")
    ),
    tag = "Authentication"
)]
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RefreshRequest>,
) -> Response {
    // Verify refresh token in DB
    let session_service = SessionService::new(state.db.clone());
    let verified = match session_service
        .verify_refresh_token(&req.refresh_token)
        .await
    {
        Ok(v) => v,
        Err(_) => {
            return (StatusCode::UNAUTHORIZED, "Invalid or expired refresh token").into_response();
        }
    };

    // Check user is active
    let user = match User::find_by_id(verified.user_id).one(&state.db).await {
        Ok(Some(user)) => user,
        _ => {
            return (StatusCode::UNAUTHORIZED, "User not found").into_response();
        }
    };

    if !user.is_active {
        return (StatusCode::FORBIDDEN, "User is deactivated").into_response();
    }

    // Get fresh permissions from DB
    let permissions = match get_user_permissions(&state.db, user.id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to get user permissions: {:?}", e);
            vec![]
        }
    };

    // Issue new JWT access token
    let auth_method = verified.auth_method.kind();
    let oidc_provider_id = verified.auth_method.oidc_provider_id();

    let access_token =
        match state
            .jwt
            .create_access_token(user.id, &permissions, auth_method, oidc_provider_id)
        {
            Ok(token) => token,
            Err(e) => {
                tracing::error!("Failed to create access token: {:?}", e);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    let response = RefreshResponse {
        access_token,
        expires_in: state.jwt.expires_in(),
        token_type: "Bearer".to_string(),
    };

    (StatusCode::OK, Json(response)).into_response()
}

// Helper functions

async fn assign_admin_role(
    db: &DatabaseConnection,
    user_id: uuid::Uuid,
) -> crate::auth::Result<()> {
    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AuthError::Internal("admin role not found".to_string())))?;

    let now = OffsetDateTime::now_utc();

    let user_role_model = user_role::ActiveModel {
        user_id: Set(user_id),
        role_id: Set(admin_role.id),
        assigned_at: Set(now),
    };

    user_role_model.insert(db).await.context_to()?;

    Ok(())
}

pub async fn assign_user_role(
    db: &DatabaseConnection,
    user_id: uuid::Uuid,
) -> crate::auth::Result<()> {
    let user_role_entity = Role::find()
        .filter(role::Column::Name.eq("user"))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AuthError::Internal("user role not found".to_string())))?;

    let now = OffsetDateTime::now_utc();

    let user_role_model = user_role::ActiveModel {
        user_id: Set(user_id),
        role_id: Set(user_role_entity.id),
        assigned_at: Set(now),
    };

    user_role_model.insert(db).await.context_to()?;

    Ok(())
}

/// Resolve the deduplicated set of permissions for a user via user_roles -> role_permissions -> permissions.
pub async fn get_user_permissions(
    db: &DatabaseConnection,
    user_id: uuid::Uuid,
) -> crate::auth::Result<Vec<Permission>> {
    // Get user's role IDs
    let user_roles = UserRole::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .all(db)
        .await
        .context_to()?;

    let role_ids: Vec<uuid::Uuid> = user_roles.iter().map(|ur| ur.role_id).collect();

    if role_ids.is_empty() {
        return Ok(vec![]);
    }

    // Get permission IDs for those roles
    let role_perms = RolePermission::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids))
        .all(db)
        .await
        .context_to()?;

    let perm_ids: Vec<uuid::Uuid> = role_perms.iter().map(|rp| rp.permission_id).collect();

    if perm_ids.is_empty() {
        return Ok(vec![]);
    }

    // Get permission names
    let perm_models = uptrakit_shared_db::entity::prelude::Permission::find()
        .filter(permission::Column::Id.is_in(perm_ids))
        .all(db)
        .await
        .context_to()?;

    // Deduplicate and convert to enum
    let mut seen = std::collections::HashSet::new();
    let permissions: Vec<Permission> = perm_models
        .into_iter()
        .filter_map(|p| Permission::parse(&p.name))
        .filter(|p| seen.insert(p.clone()))
        .collect();

    Ok(permissions)
}
