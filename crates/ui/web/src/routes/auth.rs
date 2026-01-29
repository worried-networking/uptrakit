use crate::AppState;
use crate::auth::{
    AuthError, password, registration, session::SessionService, token::generate_uuid,
};
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
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{prelude::*, role, user, user_role};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
#[schema(example = json!({
    "email": "admin@example.com",
    "first_name": "Admin",
    "last_name": "User",
    "password": "SecurePass123"
}))]
pub struct RegisterRequest {
    #[schema(example = "admin@example.com")]
    pub email: String,
    #[schema(example = "Admin")]
    pub first_name: String,
    #[schema(example = "User")]
    pub last_name: String,
    #[schema(example = "SecurePass123", min_length = 8)]
    pub password: String,
    /// Required when registration mode is `invite`.
    pub registration_token: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[schema(example = json!({
    "email": "admin@example.com",
    "password": "SecurePass123"
}))]
pub struct LoginRequest {
    #[schema(example = "admin@example.com")]
    pub email: String,
    #[schema(example = "SecurePass123")]
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserResponse,
}

#[derive(Serialize, ToSchema)]
pub struct UserResponse {
    pub id: String,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub roles: Vec<String>,
}

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
    // Validate password length
    if req.password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            "Password must be at least 8 characters",
        )
            .into_response();
    }

    // Validate registration is allowed
    {
        let settings = state.registration.read().await;
        if let Err((status, msg)) =
            registration::validate_registration(&settings, req.registration_token.as_deref())
        {
            return (status, msg).into_response();
        }
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

    if existing.is_ok() && existing.unwrap().is_some() {
        return (StatusCode::CONFLICT, "Email already exists").into_response();
    }

    // Check if this is the first user
    let user_count = User::find().count(&state.db).await.unwrap_or(0);
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
        if let Err(e) = registration::complete_initial_setup(&state.db, &state.registration).await {
            tracing::error!("Failed to complete initial registration setup: {:?}", e);
        }
    }

    // Get user roles
    let roles = match get_user_roles(&state.db, user_id).await {
        Ok(roles) => roles,
        Err(e) => {
            tracing::error!("Failed to get user roles: {:?}", e);
            vec![]
        }
    };

    // Create session
    let session_service = SessionService::new(state.db.clone());
    let token = match session_service
        .create_session(user_id, "password".to_string(), None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create session: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response = AuthResponse {
        token,
        user: UserResponse {
            id: user_id.to_string(),
            email: req.email,
            first_name: req.first_name,
            last_name: req.last_name,
            roles,
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

    // Get user roles
    let roles = match get_user_roles(&state.db, user.id).await {
        Ok(roles) => roles,
        Err(e) => {
            tracing::error!("Failed to get user roles: {:?}", e);
            vec![]
        }
    };

    // Create session
    let session_service = SessionService::new(state.db.clone());
    let token = match session_service
        .create_session(user.id, "password".to_string(), None, None)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to create session: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response = AuthResponse {
        token,
        user: UserResponse {
            id: user.id.to_string(),
            email: user.email,
            first_name: user.first_name,
            last_name: user.last_name,
            roles,
        },
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Logout and invalidate session
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    responses(
        (status = 204, description = "Logout successful"),
        (status = 401, description = "Not authenticated")
    ),
    tag = "Authentication",
    security(("bearer_token" = []))
)]
pub async fn logout(State(state): State<Arc<AppState>>, req: axum::extract::Request) -> Response {
    // Get bearer token from Authorization header
    let token = match extract_bearer_token(&req) {
        Some(token) => token,
        None => {
            return (StatusCode::UNAUTHORIZED, "Not authenticated").into_response();
        }
    };

    // Delete session
    let session_service = SessionService::new(state.db.clone());
    if let Err(e) = session_service.delete_session(&token).await {
        tracing::error!("Failed to delete session: {:?}", e);
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
pub async fn me(State(state): State<Arc<AppState>>, req: axum::extract::Request) -> Response {
    // Get bearer token from Authorization header
    let token = match extract_bearer_token(&req) {
        Some(token) => token,
        None => {
            return (StatusCode::UNAUTHORIZED, "Not authenticated").into_response();
        }
    };

    // Verify session
    let session_service = SessionService::new(state.db.clone());
    let user_id = match session_service.verify_session(&token).await {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::UNAUTHORIZED, "Not authenticated").into_response();
        }
    };

    // Get user info
    let user = match User::find_by_id(user_id).one(&state.db).await {
        Ok(Some(user)) => user,
        _ => {
            return (StatusCode::UNAUTHORIZED, "User not found").into_response();
        }
    };

    if !user.is_active {
        return (StatusCode::FORBIDDEN, "User is deactivated").into_response();
    }

    // Get user roles
    let roles = match get_user_roles(&state.db, user.id).await {
        Ok(roles) => roles,
        Err(e) => {
            tracing::error!("Failed to get user roles: {:?}", e);
            vec![]
        }
    };

    let response = UserResponse {
        id: user.id.to_string(),
        email: user.email,
        first_name: user.first_name,
        last_name: user.last_name,
        roles,
    };

    (StatusCode::OK, Json(response)).into_response()
}

// Helper functions

fn extract_bearer_token(req: &axum::extract::Request) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

async fn assign_admin_role(
    db: &DatabaseConnection,
    user_id: uuid::Uuid,
) -> crate::auth::Result<()> {
    // Get admin role
    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| AuthError::Internal("admin role not found".to_string()))?;

    let now = OffsetDateTime::now_utc();

    // Assign role to user
    let user_role_model = user_role::ActiveModel {
        user_id: Set(user_id),
        role_id: Set(admin_role.id),
        assigned_at: Set(now),
    };

    user_role_model.insert(db).await.context_to()?;

    Ok(())
}

async fn get_user_roles(
    db: &DatabaseConnection,
    user_id: uuid::Uuid,
) -> crate::auth::Result<Vec<String>> {
    let user_roles = UserRole::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .find_also_related(Role)
        .all(db)
        .await
        .context_to()?;

    let roles = user_roles
        .into_iter()
        .filter_map(|(_, role)| role.map(|r| r.name))
        .collect();

    Ok(roles)
}
