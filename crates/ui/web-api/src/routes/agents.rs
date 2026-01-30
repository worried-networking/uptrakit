use crate::AppState;
use crate::auth::{password, token};
use crate::extract::ClientIp;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::settings_store::{delete_setting, load_setting, upsert_setting};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_shared_db::entity::{agent, prelude::*};
use utoipa::ToSchema;

const SETTING_KEY_ENROLLMENT_TOKEN_HASH: &str = "agent_enrollment.token_hash";

// --- Agent status enum ---

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Pending,
    Approved,
    Rejected,
}

impl AgentStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

// --- Request/Response types ---

#[derive(Deserialize, ToSchema)]
pub struct EnrollRequest {
    pub hostname: String,
    pub friendly_name: String,
    pub enrollment_token: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct EnrollResponse {
    pub agent_id: String,
    pub status: AgentStatus,
    pub enrollment_secret: String,
}

#[derive(Serialize, ToSchema)]
pub struct EnrollStatusResponse {
    pub agent_id: String,
    pub status: AgentStatus,
}

#[derive(Serialize, ToSchema)]
pub struct AgentResponse {
    pub id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub ip_address: Option<String>,
    pub status: AgentStatus,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ListAgentsQuery {
    pub status: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct EnrollmentTokenResponse {
    pub token: String,
}

#[derive(Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

// --- Agent-facing endpoints (no user auth) ---

/// Agent requests enrollment
#[utoipa::path(
    post,
    path = "/api/v1/agents/enroll",
    request_body = EnrollRequest,
    responses(
        (status = 201, description = "Enrollment request created", body = EnrollResponse),
        (status = 400, description = "Invalid request"),
        (status = 403, description = "Invalid enrollment token")
    ),
    tag = "Agents"
)]
pub async fn enroll(
    State(state): State<Arc<AppState>>,
    client_ip: Option<Extension<ClientIp>>,
    Json(req): Json<EnrollRequest>,
) -> Response {
    // Validate hostname non-empty
    if req.hostname.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "hostname must not be empty").into_response();
    }

    // Determine status based on enrollment token
    let status = if let Some(ref enrollment_token) = req.enrollment_token {
        // Verify against stored Argon2 hash
        let token_hash = match load_setting(&state.db, SETTING_KEY_ENROLLMENT_TOKEN_HASH).await {
            Ok(Some(hash)) => hash,
            Ok(None) => {
                return (StatusCode::FORBIDDEN, "No enrollment token configured").into_response();
            }
            Err(e) => {
                tracing::error!("Failed to load enrollment token hash: {:?}", e);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

        match password::verify_password(enrollment_token, &token_hash) {
            Ok(true) => AgentStatus::Approved,
            Ok(false) => {
                return (StatusCode::FORBIDDEN, "Invalid enrollment token").into_response();
            }
            Err(e) => {
                tracing::error!("Token verification error: {:?}", e);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    } else {
        AgentStatus::Pending
    };

    // Generate agent ID, enrollment secret
    let agent_id = token::generate_uuid();
    let enrollment_secret = match token::generate_secure_token() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to generate enrollment secret: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let secret_hash = token::hash_token(&enrollment_secret);

    // Capture IP
    let ip_address = client_ip.map(|Extension(ClientIp(ip))| ip.to_string());

    let now = OffsetDateTime::now_utc();
    let model = agent::ActiveModel {
        id: Set(agent_id),
        hostname: Set(req.hostname),
        friendly_name: Set(req.friendly_name),
        ip_address: Set(ip_address),
        status: Set(status.as_str().to_string()),
        enrollment_secret_hash: Set(secret_hash),
        last_seen_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    if let Err(e) = model.insert(&state.db).await {
        tracing::error!("Failed to insert agent: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let response = EnrollResponse {
        agent_id: agent_id.to_string(),
        status,
        enrollment_secret,
    };

    (StatusCode::CREATED, Json(response)).into_response()
}

/// Agent polls enrollment status
#[utoipa::path(
    get,
    path = "/api/v1/agents/enroll/status",
    responses(
        (status = 200, description = "Current enrollment status", body = EnrollStatusResponse),
        (status = 401, description = "Invalid enrollment secret")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn enroll_status(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response {
    // Extract bearer token
    let enrollment_secret = match extract_bearer_token(&req) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Missing enrollment secret").into_response(),
    };

    let secret_hash = token::hash_token(&enrollment_secret);

    // Look up agent by enrollment_secret_hash, excluding deactivated
    let agent = match Agent::find()
        .filter(agent::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(agent::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "Invalid enrollment secret").into_response(),
        Err(e) => {
            tracing::error!("DB error looking up agent: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Update last_seen_at
    let now = OffsetDateTime::now_utc();
    let mut active: agent::ActiveModel = agent.clone().into();
    active.last_seen_at = Set(Some(now));
    active.updated_at = Set(now);
    if let Err(e) = active.update(&state.db).await {
        tracing::error!("Failed to update agent last_seen_at: {}", e);
    }

    let status = AgentStatus::from_str(&agent.status).unwrap_or(AgentStatus::Pending);
    let response = EnrollStatusResponse {
        agent_id: agent.id.to_string(),
        status,
    };

    (StatusCode::OK, Json(response)).into_response()
}

// --- Admin-facing endpoints (user auth + admin role) ---

/// List all agents
#[utoipa::path(
    get,
    path = "/api/v1/agents",
    params(
        ("status" = Option<String>, Query, description = "Filter by status (pending, approved, rejected)")
    ),
    responses(
        (status = 200, description = "List of agents", body = Vec<AgentResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<ListAgentsQuery>,
) -> Response {
    if !check_admin_role(&state.db, user.user_id).await {
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }

    let mut q = Agent::find().filter(agent::Column::DeactivatedAt.is_null());

    if let Some(ref status) = query.status {
        q = q.filter(agent::Column::Status.eq(status.as_str()));
    }

    let agents = match q
        .order_by_desc(agent::Column::CreatedAt)
        .all(&state.db)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to list agents: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response: Vec<AgentResponse> = agents.into_iter().map(agent_to_response).collect();
    (StatusCode::OK, Json(response)).into_response()
}

/// Approve a pending agent
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/approve",
    params(
        ("id" = String, Path, description = "Agent UUID")
    ),
    responses(
        (status = 200, description = "Agent approved", body = AgentResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Agent not found")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn approve_agent(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !check_admin_role(&state.db, user.user_id).await {
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }

    let agent_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid agent ID").into_response(),
    };

    let agent = match Agent::find_by_id(agent_id)
        .filter(agent::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "Agent not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if agent.status != AgentStatus::Pending.as_str() {
        return (StatusCode::BAD_REQUEST, "Agent is not in pending status").into_response();
    }

    let now = OffsetDateTime::now_utc();
    let mut active: agent::ActiveModel = agent.into();
    active.status = Set(AgentStatus::Approved.as_str().to_string());
    active.updated_at = Set(now);

    let updated = match active.update(&state.db).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to approve agent: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    (StatusCode::OK, Json(agent_to_response(updated))).into_response()
}

/// Reject a pending agent
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/reject",
    params(
        ("id" = String, Path, description = "Agent UUID")
    ),
    responses(
        (status = 200, description = "Agent rejected", body = AgentResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Agent not found")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn reject_agent(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !check_admin_role(&state.db, user.user_id).await {
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }

    let agent_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid agent ID").into_response(),
    };

    let agent = match Agent::find_by_id(agent_id)
        .filter(agent::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "Agent not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if agent.status != AgentStatus::Pending.as_str() {
        return (StatusCode::BAD_REQUEST, "Agent is not in pending status").into_response();
    }

    let now = OffsetDateTime::now_utc();
    let mut active: agent::ActiveModel = agent.into();
    active.status = Set(AgentStatus::Rejected.as_str().to_string());
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    let updated = match active.update(&state.db).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to reject agent: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    (StatusCode::OK, Json(agent_to_response(updated))).into_response()
}

/// Deactivate an agent (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/agents/{id}",
    params(
        ("id" = String, Path, description = "Agent UUID")
    ),
    responses(
        (status = 200, description = "Agent deactivated", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Agent not found")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn deactivate_agent(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !check_admin_role(&state.db, user.user_id).await {
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }

    let agent_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid agent ID").into_response(),
    };

    let agent = match Agent::find_by_id(agent_id)
        .filter(agent::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "Agent not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut active: agent::ActiveModel = agent.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    if let Err(e) = active.update(&state.db).await {
        tracing::error!("Failed to deactivate agent: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: "Agent deactivated".to_string(),
        }),
    )
        .into_response()
}

/// Generate a new enrollment token
#[utoipa::path(
    post,
    path = "/api/v1/agents/enrollment-token",
    responses(
        (status = 201, description = "Enrollment token generated", body = EnrollmentTokenResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn create_enrollment_token(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !check_admin_role(&state.db, user.user_id).await {
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }

    let plaintext = match token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to generate enrollment token: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let hash = match password::hash_password(&plaintext) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to hash enrollment token: {:?}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(e) = upsert_setting(&state.db, SETTING_KEY_ENROLLMENT_TOKEN_HASH, &hash).await {
        tracing::error!("Failed to store enrollment token hash: {:?}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (
        StatusCode::CREATED,
        Json(EnrollmentTokenResponse { token: plaintext }),
    )
        .into_response()
}

/// Revoke the enrollment token
#[utoipa::path(
    delete,
    path = "/api/v1/agents/enrollment-token",
    responses(
        (status = 200, description = "Enrollment token revoked", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn revoke_enrollment_token(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !check_admin_role(&state.db, user.user_id).await {
        return (StatusCode::FORBIDDEN, "Admin role required").into_response();
    }

    if let Err(e) = delete_setting(&state.db, SETTING_KEY_ENROLLMENT_TOKEN_HASH).await {
        tracing::error!("Failed to delete enrollment token: {:?}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: "Enrollment token revoked".to_string(),
        }),
    )
        .into_response()
}

// --- Helper functions ---

fn extract_bearer_token(req: &axum::extract::Request) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

fn format_rfc3339(dt: OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

fn agent_to_response(agent: agent::Model) -> AgentResponse {
    AgentResponse {
        id: agent.id.to_string(),
        hostname: agent.hostname,
        friendly_name: agent.friendly_name,
        ip_address: agent.ip_address,
        status: AgentStatus::from_str(&agent.status).unwrap_or(AgentStatus::Pending),
        last_seen_at: agent.last_seen_at.map(format_rfc3339),
        created_at: format_rfc3339(agent.created_at),
        updated_at: format_rfc3339(agent.updated_at),
    }
}

async fn check_admin_role(db: &sea_orm::DatabaseConnection, user_id: uuid::Uuid) -> bool {
    use uptrakit_shared_db::entity::{prelude::*, user_role};

    UserRole::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .find_also_related(Role)
        .all(db)
        .await
        .map(|roles| {
            roles
                .iter()
                .any(|(_, r)| r.as_ref().is_some_and(|r| r.name == "admin"))
        })
        .unwrap_or(false)
}
