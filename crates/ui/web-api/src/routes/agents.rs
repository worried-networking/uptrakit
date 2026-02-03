use crate::AppState;
use crate::SettingKey;
use crate::auth::permissions::Permission;
use crate::auth::{password, token};
use crate::cert_signer::SignedCertBundle;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use crate::settings_store::{delete_setting, load_setting, upsert_setting};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rootcause::{Report, ReportConversion, markers, prelude::*};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, sea_query::Expr,
};
use std::net::IpAddr;
use std::sync::Arc;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_internal_wire::{ApprovedPayload, ControllerMessage, HostInfo, RejectedPayload};
use uptrakit_shared_db::entity::prelude::RevocationReason;
use uptrakit_shared_db::entity::{agent, agent_certificate, agent_host, host, prelude::*};

pub use uptrakit_web_api_types::agents::{
    AgentResponse, AgentStatus, EnrollmentTokenResponse, EnrollmentTokenStatusResponse,
    ListAgentsQuery, MergeAgentRequest, MessageResponse,
};

// --- Agent route error type ---

#[derive(Debug, Error)]
pub(crate) enum AgentRouteError {
    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    Forbidden(String),

    #[error("{0}")]
    NotFound(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("certificate signing error")]
    CertSigning,

    #[error("client_id collision")]
    ClientIdCollision,
}

impl AgentRouteError {
    pub(crate) fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::ClientIdCollision => StatusCode::CONFLICT,
            Self::Internal(_) | Self::Database(_) | Self::CertSigning => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl<T> ReportConversion<sea_orm::DbErr, markers::Mutable, T> for AgentRouteError
where
    AgentRouteError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<sea_orm::DbErr, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(AgentRouteError::Database)
    }
}

// --- Shared enrollment helpers (used by both WS handler and admin endpoints) ---

/// Result of a successful enrollment.
pub(crate) struct EnrollResult {
    pub agent: agent::Model,
    pub enrollment_secret: String,
    pub status: AgentStatus,
}

/// Core enrollment logic: creates agent record, returns model + plaintext secret.
///
/// The `client_id` is an agent-generated UUIDv7. If an active agent with that
/// UUID already exists, enrollment is rejected with `ClientIdCollision`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_enroll(
    db: &sea_orm::DatabaseConnection,
    settings: &crate::settings::Settings,
    tenant_id: uuid::Uuid,
    client_id: &str,
    hostname: &str,
    friendly_name: &str,
    enrollment_token: Option<&str>,
    ip_address: Option<IpAddr>,
    host_info: Option<&HostInfo>,
) -> Result<EnrollResult, Report<AgentRouteError>> {
    if hostname.trim().is_empty() {
        return Err(report!(AgentRouteError::BadRequest(
            "hostname must not be empty".into()
        )));
    }

    // Parse client_id as UUID
    let agent_id = uuid::Uuid::parse_str(client_id).map_err(|e| {
        report!(AgentRouteError::BadRequest(format!(
            "invalid client_id: {e}"
        )))
    })?;

    // Collision detection: reject if an active agent with this UUID exists in this tenant
    let existing = Agent::find_by_id(agent_id)
        .filter(agent::Column::TenantId.eq(tenant_id))
        .filter(agent::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to::<AgentRouteError>()?;

    if existing.is_some() {
        return Err(report!(AgentRouteError::ClientIdCollision));
    }

    // Determine status based on enrollment token
    let status = if let Some(enrollment_token) = enrollment_token {
        let token_hash = match load_setting(db, tenant_id, SettingKey::EnrollmentTokenHash).await {
            Ok(Some(v)) => match v.as_str() {
                Some(hash) => hash.to_string(),
                None => {
                    return Err(report!(AgentRouteError::Forbidden(
                        "No enrollment token configured".into()
                    )));
                }
            },
            Ok(None) => {
                return Err(report!(AgentRouteError::Forbidden(
                    "No enrollment token configured".into()
                )));
            }
            Err(e) => {
                tracing::error!("Failed to load enrollment token hash: {:?}", e);
                return Err(report!(AgentRouteError::Internal(
                    "Internal server error".into()
                )));
            }
        };

        match password::verify_password(enrollment_token, &token_hash) {
            Ok(true) => AgentStatus::Approved,
            Ok(false) => {
                return Err(report!(AgentRouteError::Forbidden(
                    "Invalid enrollment token".into()
                )));
            }
            Err(e) => {
                tracing::error!("Token verification error: {:?}", e);
                return Err(report!(AgentRouteError::Internal(
                    "Internal server error".into()
                )));
            }
        }
    } else {
        AgentStatus::Pending
    };

    let enrollment_secret = match token::generate_secure_token() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to generate enrollment secret: {:?}", e);
            return Err(report!(AgentRouteError::Internal(
                "Internal server error".into()
            )));
        }
    };
    let secret_hash = token::hash_token(&enrollment_secret);

    let ip_str = ip_address.map(|ip| ip.to_string());
    let _ = settings; // settings available for future use

    let now = OffsetDateTime::now_utc();
    let model = agent::ActiveModel {
        id: Set(agent_id),
        tenant_id: Set(tenant_id),
        hostname: Set(hostname.to_string()),
        friendly_name: Set(friendly_name.to_string()),
        ip_address: Set(ip_str.clone()),
        status: Set(status.as_str().to_string()),
        enrollment_secret_hash: Set(secret_hash),
        agent_version: Set("unknown".to_string()),
        last_seen_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let inserted = model.insert(db).await.context_to::<AgentRouteError>()?;

    // Link agent to host (non-fatal on failure)
    if let Some(info) = host_info
        && let Err(e) = find_or_create_host_and_link(
            db,
            tenant_id,
            inserted.id,
            info,
            hostname,
            ip_str.as_deref(),
        )
        .await
    {
        tracing::warn!(error = %e, "failed to link agent to host during enrollment");
    }

    Ok(EnrollResult {
        agent: inserted,
        enrollment_secret,
        status,
    })
}

/// Find or create a host by machine_id, then link it to the given agent.
///
/// Skips silently when `machine_id == "unknown"`.
pub(crate) async fn find_or_create_host_and_link(
    db: &sea_orm::DatabaseConnection,
    tenant_id: uuid::Uuid,
    agent_id: uuid::Uuid,
    host_info: &HostInfo,
    hostname: &str,
    ip_address: Option<&str>,
) -> Result<(), Report<AgentRouteError>> {
    if host_info.machine_id == "unknown" {
        return Ok(());
    }

    let now = OffsetDateTime::now_utc();

    let existing = Host::find()
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::MachineId.eq(&host_info.machine_id))
        .one(db)
        .await
        .context_to::<AgentRouteError>()?;

    let host_id = if let Some(existing_host) = existing {
        // Update mutable fields
        let mut active: host::ActiveModel = existing_host.clone().into();
        active.hostname = Set(hostname.to_string());
        if let Some(ip) = ip_address {
            active.ip_address = Set(Some(ip.to_string()));
        }
        if let Some(ref os_type) = host_info.os_type {
            active.os_type = Set(Some(os_type.clone()));
        }
        if let Some(ref os_version) = host_info.os_version {
            active.os_version = Set(Some(os_version.clone()));
        }
        if let Some(ref architecture) = host_info.architecture {
            active.architecture = Set(Some(architecture.clone()));
        }
        active.last_seen_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(db).await.context_to::<AgentRouteError>()?;
        existing_host.id
    } else {
        // Create new host
        let host_id = token::generate_uuid();
        let new_host = host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(host_info.machine_id.clone()),
            hostname: Set(hostname.to_string()),
            friendly_name: Set(hostname.to_string()),
            os_type: Set(host_info.os_type.clone()),
            os_version: Set(host_info.os_version.clone()),
            architecture: Set(host_info.architecture.clone()),
            ip_address: Set(ip_address.map(|s| s.to_string())),
            last_seen_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        new_host.insert(db).await.context_to::<AgentRouteError>()?;
        host_id
    };

    // Upsert agent_host link — insert if not exists
    let existing_link = AgentHost::find_by_id((agent_id, host_id))
        .one(db)
        .await
        .context_to::<AgentRouteError>()?;

    if existing_link.is_none() {
        let link = agent_host::ActiveModel {
            agent_id: Set(agent_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        };
        link.insert(db).await.context_to::<AgentRouteError>()?;
    }

    Ok(())
}

/// Look up an agent by hashed enrollment_secret.
pub(crate) async fn do_lookup_by_secret(
    db: &sea_orm::DatabaseConnection,
    enrollment_secret: &str,
) -> Result<agent::Model, Report<AgentRouteError>> {
    let secret_hash = token::hash_token(enrollment_secret);

    let agent = Agent::find()
        .filter(agent::Column::EnrollmentSecretHash.eq(&secret_hash))
        .filter(agent::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to::<AgentRouteError>()?;

    agent.ok_or_else(|| {
        report!(AgentRouteError::NotFound(
            "Invalid enrollment secret".into()
        ))
    })
}

/// Sign a certificate from the agent's CSR, invalidate enrollment secret.
pub(crate) async fn do_sign_csr(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    agent: agent::Model,
    csr_pem: &str,
) -> Result<SignedCertBundle, Report<AgentRouteError>> {
    if agent.status != AgentStatus::Approved.as_str() {
        return Err(report!(AgentRouteError::Forbidden(
            "Agent is not approved".into()
        )));
    }

    let lifetime = time::Duration::days(i64::from(settings.agent_cert_lifetime_days().await));

    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &agent.id, lifetime)
        .map_err(|e| {
            tracing::error!("Failed to sign agent certificate: {e}");
            report!(AgentRouteError::CertSigning)
        })?;

    // Record certificate in DB for revocation tracking
    if let Err(e) = record_certificate(db, agent.id, &bundle.cert_pem, &ca_fp).await {
        tracing::error!("Failed to record agent certificate: {:?}", e);
        return Err(report!(AgentRouteError::Internal(
            "Internal server error".into()
        )));
    }

    // Invalidate enrollment secret
    let invalidated_hash = token::hash_token(&token::generate_uuid().to_string());
    let now = OffsetDateTime::now_utc();
    let mut active: agent::ActiveModel = agent.into();
    active.enrollment_secret_hash = Set(invalidated_hash);
    active.last_seen_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(db).await.context_to::<AgentRouteError>()?;

    Ok(bundle)
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
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<ListAgentsQuery>,
) -> Response {
    if !user.has_permission(Permission::ViewAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let mut q = Agent::find()
        .filter(agent::Column::TenantId.eq(tenant.tenant_id))
        .filter(agent::Column::DeactivatedAt.is_null());

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
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let agent_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid agent ID").into_response(),
    };

    let agent = match Agent::find_by_id(agent_id)
        .filter(agent::Column::TenantId.eq(tenant.tenant_id))
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

    // Push approval to connected agent via WebSocket
    let _ = state
        .agent_connections
        .send(
            &agent_id,
            ControllerMessage::Approved(ApprovedPayload {
                agent_id: agent_id.to_string(),
            }),
        )
        .await;

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
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let agent_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid agent ID").into_response(),
    };

    let agent = match Agent::find_by_id(agent_id)
        .filter(agent::Column::TenantId.eq(tenant.tenant_id))
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

    // Push rejection to connected agent via WebSocket
    let _ = state
        .agent_connections
        .send(
            &agent_id,
            ControllerMessage::Rejected(RejectedPayload {
                agent_id: agent_id.to_string(),
            }),
        )
        .await;

    // Terminate any active WebSocket connection for this agent
    state.agent_connections.unregister(&agent_id).await;

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
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let agent_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid agent ID").into_response(),
    };

    let agent = match Agent::find_by_id(agent_id)
        .filter(agent::Column::TenantId.eq(tenant.tenant_id))
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

    // Revoke all non-revoked certificates for this agent
    let now = OffsetDateTime::now_utc();
    if let Err(e) = AgentCertificate::update_many()
        .col_expr(agent_certificate::Column::RevokedAt, Expr::value(Some(now)))
        .col_expr(
            agent_certificate::Column::RevocationReason,
            Expr::value(Some(RevocationReason::AgentDeactivated)),
        )
        .filter(agent_certificate::Column::AgentId.eq(agent_id))
        .filter(agent_certificate::Column::RevokedAt.is_null())
        .exec(&state.db)
        .await
    {
        tracing::error!("Failed to revoke certificates: {}", e);
    }

    state.revocation_notify.notify_one();

    // Terminate any active WebSocket connection for this agent.
    // Dropping the sender causes the handler's push_rx.recv() to
    // return None, which breaks the select loop and closes the socket.
    state.agent_connections.unregister(&agent_id).await;

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
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
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

    if let Err(e) = upsert_setting(
        &state.db,
        state.default_tenant_id,
        SettingKey::EnrollmentTokenHash,
        serde_json::Value::String(hash),
    )
    .await
    {
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
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    if let Err(e) = delete_setting(
        &state.db,
        state.default_tenant_id,
        SettingKey::EnrollmentTokenHash,
    )
    .await
    {
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

/// Check if an enrollment token is configured
#[utoipa::path(
    get,
    path = "/api/v1/agents/enrollment-token/status",
    responses(
        (status = 200, description = "Enrollment token status", body = EnrollmentTokenStatusResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn enrollment_token_status(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let configured = matches!(
        load_setting(
            &state.db,
            state.default_tenant_id,
            SettingKey::EnrollmentTokenHash
        )
        .await,
        Ok(Some(_))
    );

    (
        StatusCode::OK,
        Json(EnrollmentTokenStatusResponse { configured }),
    )
        .into_response()
}

// --- Helper functions ---

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
        agent_version: agent.agent_version,
        last_seen_at: agent.last_seen_at.map(format_rfc3339),
        created_at: format_rfc3339(agent.created_at),
        updated_at: format_rfc3339(agent.updated_at),
    }
}

// --- Certificate recording error type ---

#[derive(Debug, Error)]
pub(crate) enum CertRecordError {
    #[error("failed to parse PEM data")]
    PemParse,

    #[error("failed to parse X.509 certificate")]
    X509Parse,

    #[error("invalid certificate timestamp: {0}")]
    Timestamp(#[from] time::error::ComponentRange),

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

impl<T> ReportConversion<sea_orm::DbErr, markers::Mutable, T> for CertRecordError
where
    CertRecordError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<sea_orm::DbErr, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(CertRecordError::Database)
    }
}

impl<T> ReportConversion<time::error::ComponentRange, markers::Mutable, T> for CertRecordError
where
    CertRecordError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<time::error::ComponentRange, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(CertRecordError::Timestamp)
    }
}

async fn record_certificate(
    db: &sea_orm::DatabaseConnection,
    agent_id: uuid::Uuid,
    cert_pem: &str,
    ca_fingerprint: &str,
) -> Result<(), Report<CertRecordError>> {
    let (serial, not_before, not_after) = parse_cert_metadata(cert_pem)?;

    let record = agent_certificate::ActiveModel {
        ca_fingerprint: Set(ca_fingerprint.to_string()),
        serial_number: Set(serial),
        agent_id: Set(agent_id),
        not_before: Set(not_before),
        not_after: Set(not_after),
        revoked_at: Set(None),
        revocation_reason: Set(None),
        created_at: Set(OffsetDateTime::now_utc()),
        last_seen_at: Set(None),
    };

    record.insert(db).await.context_to::<CertRecordError>()?;

    Ok(())
}

/// Revoke a certificate by serial number and CA fingerprint.
pub(crate) async fn revoke_certificate(
    db: &sea_orm::DatabaseConnection,
    serial_number: &str,
    ca_fingerprint: &str,
    reason: RevocationReason,
) -> Result<(), Report<CertRecordError>> {
    AgentCertificate::update_many()
        .col_expr(
            agent_certificate::Column::RevokedAt,
            Expr::value(Some(OffsetDateTime::now_utc())),
        )
        .col_expr(
            agent_certificate::Column::RevocationReason,
            Expr::value(Some(reason)),
        )
        .filter(agent_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .filter(agent_certificate::Column::SerialNumber.eq(serial_number))
        .filter(agent_certificate::Column::RevokedAt.is_null())
        .exec(db)
        .await
        .context_to::<CertRecordError>()?;
    Ok(())
}

fn parse_cert_metadata(
    pem: &str,
) -> Result<(String, OffsetDateTime, OffsetDateTime), Report<CertRecordError>> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| report!(CertRecordError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(CertRecordError::X509Parse))?;

    let serial = cert.raw_serial_as_string();
    let validity = cert.validity();
    let not_before = OffsetDateTime::from_unix_timestamp(validity.not_before.timestamp())
        .context_to::<CertRecordError>()?;
    let not_after = OffsetDateTime::from_unix_timestamp(validity.not_after.timestamp())
        .context_to::<CertRecordError>()?;

    Ok((serial, not_before, not_after))
}

// --- Merge endpoint ---

/// Merge a pending (source) agent into an existing approved (target) agent.
#[utoipa::path(
    post,
    path = "/api/v1/agents/{target_id}/merge",
    params(
        ("target_id" = String, Path, description = "Target agent UUID (approved)")
    ),
    request_body = MergeAgentRequest,
    responses(
        (status = 200, description = "Agents merged", body = AgentResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Agent not found")
    ),
    tag = "Agents",
    security(("bearer_token" = []))
)]
pub async fn merge_agent(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(target_id): Path<String>,
    Json(body): Json<MergeAgentRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let target_uuid = match uuid::Uuid::parse_str(&target_id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid target agent ID").into_response(),
    };

    let source_uuid = match uuid::Uuid::parse_str(&body.source_id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid source agent ID").into_response(),
    };

    if target_uuid == source_uuid {
        return (StatusCode::BAD_REQUEST, "Cannot merge agent into itself").into_response();
    }

    // Find target agent (must be approved, not deactivated)
    let target = match Agent::find_by_id(target_uuid)
        .filter(agent::Column::TenantId.eq(tenant.tenant_id))
        .filter(agent::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "Target agent not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if target.status != AgentStatus::Approved.as_str() {
        return (StatusCode::BAD_REQUEST, "Target agent must be approved").into_response();
    }

    if state.agent_connections.is_connected(&target_uuid).await {
        return (StatusCode::CONFLICT, "Target agent is currently connected").into_response();
    }

    // Find source agent (must be pending, not deactivated)
    let source = match Agent::find_by_id(source_uuid)
        .filter(agent::Column::TenantId.eq(tenant.tenant_id))
        .filter(agent::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "Source agent not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if source.status != AgentStatus::Pending.as_str() {
        return (StatusCode::BAD_REQUEST, "Source agent must be pending").into_response();
    }

    let now = OffsetDateTime::now_utc();

    // Save the hash before deactivating the source
    let source_secret_hash = source.enrollment_secret_hash.clone();
    let source_hostname = source.hostname.clone();
    let source_friendly_name = source.friendly_name.clone();
    let source_ip_address = source.ip_address.clone();

    // Deactivate source first — invalidate its hash to free the unique constraint
    let invalidated_hash = token::hash_token(&token::generate_uuid().to_string());
    let mut source_active: agent::ActiveModel = source.into();
    source_active.enrollment_secret_hash = Set(invalidated_hash);
    source_active.deactivated_at = Set(Some(now));
    source_active.updated_at = Set(now);

    if let Err(e) = source_active.update(&state.db).await {
        tracing::error!("Failed to deactivate source agent: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Revoke all non-revoked certificates for both agents.
    // Source is being absorbed; target will get a fresh certificate via enrollment.
    for (agent_uuid, label) in [(source_uuid, "source"), (target_uuid, "target")] {
        if let Err(e) = AgentCertificate::update_many()
            .col_expr(agent_certificate::Column::RevokedAt, Expr::value(Some(now)))
            .col_expr(
                agent_certificate::Column::RevocationReason,
                Expr::value(Some(RevocationReason::AgentMerged)),
            )
            .filter(agent_certificate::Column::AgentId.eq(agent_uuid))
            .filter(agent_certificate::Column::RevokedAt.is_null())
            .exec(&state.db)
            .await
        {
            tracing::error!("Failed to revoke {label} agent certificates: {}", e);
        }
    }

    state.revocation_notify.notify_one();

    // Now copy source's enrollment_secret_hash to target
    let mut target_active: agent::ActiveModel = target.into();
    target_active.enrollment_secret_hash = Set(source_secret_hash);
    target_active.hostname = Set(source_hostname);
    target_active.friendly_name = Set(source_friendly_name);
    target_active.ip_address = Set(source_ip_address);
    target_active.updated_at = Set(now);

    let updated_target = match target_active.update(&state.db).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to update target agent: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Copy source agent's host links to target (INSERT ON CONFLICT DO NOTHING)
    if let Ok(source_links) = AgentHost::find()
        .filter(agent_host::Column::AgentId.eq(source_uuid))
        .all(&state.db)
        .await
    {
        for link in source_links {
            let existing = AgentHost::find_by_id((target_uuid, link.host_id))
                .one(&state.db)
                .await;
            if matches!(existing, Ok(None)) {
                let new_link = agent_host::ActiveModel {
                    agent_id: Set(target_uuid),
                    host_id: Set(link.host_id),
                    linked_at: Set(now),
                };
                if let Err(e) = new_link.insert(&state.db).await {
                    tracing::warn!("failed to copy host link during merge: {}", e);
                }
            }
        }
    }

    // Terminate source's WebSocket connection
    state.agent_connections.unregister(&source_uuid).await;

    tracing::info!(
        target_id = %target_uuid,
        source_id = %source_uuid,
        "agents merged: source deactivated, target updated"
    );

    (StatusCode::OK, Json(agent_to_response(updated_target))).into_response()
}
