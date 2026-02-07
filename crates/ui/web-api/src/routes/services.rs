use crate::AppState;
use crate::SettingKey;
use crate::auth::permissions::Permission;
use crate::auth::{password, token};
use crate::error_response::error_response;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use crate::settings_store::{delete_setting, load_setting, upsert_setting};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, sea_query::Expr,
};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_internal_wire::{ApprovedPayload, ControllerMessage, RejectedPayload};
use uptrakit_shared_db::entity::prelude::{
    RevocationReason, Service, ServiceCertificate, ServiceHost,
};
use uptrakit_shared_db::entity::{service, service_certificate, service_host};

pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::services::{
    EnrollmentTokenResponse, EnrollmentTokenStatusResponse, ListServicesQuery, MergeAgentRequest,
    MessageResponse, ServiceResponse, ServiceStatus, ServiceType,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_rfc3339(dt: OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

fn model_to_response(m: service::Model) -> ServiceResponse {
    let service_type = match m.service_type {
        service::ServiceType::Agent => ServiceType::Agent,
        service::ServiceType::Mqtt => ServiceType::Mqtt,
    };
    let status = match m.status {
        service::ServiceStatus::Pending => ServiceStatus::Pending,
        service::ServiceStatus::Approved => ServiceStatus::Approved,
        service::ServiceStatus::Rejected => ServiceStatus::Rejected,
        service::ServiceStatus::Deactivated => ServiceStatus::Deactivated,
    };
    ServiceResponse {
        id: m.id.to_string(),
        service_type,
        hostname: m.hostname,
        friendly_name: m.friendly_name,
        ip_address: m.ip_address,
        status,
        client_version: m.client_version,
        last_seen_at: m.last_seen_at.map(format_rfc3339),
        created_at: format_rfc3339(m.created_at),
        updated_at: format_rfc3339(m.updated_at),
    }
}

/// Parse the `type` query parameter into a DB `ServiceType`.
fn parse_service_type(s: &str) -> Option<service::ServiceType> {
    match s {
        "agent" => Some(service::ServiceType::Agent),
        "mqtt" => Some(service::ServiceType::Mqtt),
        _ => None,
    }
}

/// Parse the `status` query parameter into a DB `ServiceStatus`.
fn parse_service_status(s: &str) -> Option<service::ServiceStatus> {
    match s {
        "pending" => Some(service::ServiceStatus::Pending),
        "approved" => Some(service::ServiceStatus::Approved),
        "rejected" => Some(service::ServiceStatus::Rejected),
        "deactivated" => Some(service::ServiceStatus::Deactivated),
        _ => None,
    }
}

/// Error returned when the `type` query parameter is invalid.
#[derive(Debug, thiserror::Error)]
#[error("invalid type parameter: '{0}', expected 'agent' or 'mqtt'")]
struct InvalidServiceTypeParam(String);

impl IntoResponse for InvalidServiceTypeParam {
    fn into_response(self) -> Response {
        error_response(StatusCode::BAD_REQUEST, self.to_string())
    }
}

/// Determine the correct `SettingKey` for the enrollment token hash based on
/// the `type` query parameter.
fn enrollment_setting_key(
    type_param: Option<&str>,
) -> std::result::Result<SettingKey, InvalidServiceTypeParam> {
    match type_param {
        Some("agent") | None => Ok(SettingKey::EnrollmentTokenHash),
        Some("mqtt") => Ok(SettingKey::MqttEnrollmentTokenHash),
        Some(other) => Err(InvalidServiceTypeParam(other.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all services (agents and/or MQTT)
#[utoipa::path(
    get,
    path = "/api/v1/services",
    params(
        ("type" = Option<String>, Query, description = "Filter by service type (agent, mqtt)"),
        ("status" = Option<String>, Query, description = "Filter by status (pending, approved, rejected, deactivated)"),
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    responses(
        (status = 200, description = "Paginated list of services", body = PaginatedResponse<ServiceResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn list_services(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    if !user.has_permission(Permission::ViewAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let pagination = query.pagination().resolve();

    let mut q = Service::find()
        .filter(service::Column::TenantId.eq(tenant.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null());

    if let Some(ref type_filter) = query.r#type
        && let Some(db_type) = parse_service_type(type_filter)
    {
        q = q.filter(service::Column::ServiceType.eq(db_type));
    }

    if let Some(ref status_filter) = query.status
        && let Some(db_status) = parse_service_status(status_filter)
    {
        q = q.filter(service::Column::Status.eq(db_status));
    }

    let base_query = q.order_by_desc(service::Column::CreatedAt);

    let total = match base_query.clone().count(&state.db).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to count services: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let services = match base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(&state.db)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to list services: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let items: Vec<ServiceResponse> = services.into_iter().map(model_to_response).collect();
    (
        StatusCode::OK,
        Json(PaginatedResponse::new(items, total, pagination)),
    )
        .into_response()
}

/// Approve a pending service
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/approve",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service approved", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn approve_service(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid service ID"),
    };

    let svc = match Service::find_by_id(service_id)
        .filter(service::Column::TenantId.eq(tenant.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if svc.status != service::ServiceStatus::Pending {
        return error_response(StatusCode::BAD_REQUEST, "Service is not in pending status");
    }

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.status = Set(service::ServiceStatus::Approved);
    active.updated_at = Set(now);

    let updated = match active.update(&state.db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to approve service: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Push approval via WebSocket
    let _ = state
        .service_connections
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    (StatusCode::OK, Json(model_to_response(updated))).into_response()
}

/// Reject a pending service
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/reject",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service rejected", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn reject_service(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid service ID"),
    };

    let svc = match Service::find_by_id(service_id)
        .filter(service::Column::TenantId.eq(tenant.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if svc.status != service::ServiceStatus::Pending {
        return error_response(StatusCode::BAD_REQUEST, "Service is not in pending status");
    }

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.status = Set(service::ServiceStatus::Rejected);
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    let updated = match active.update(&state.db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to reject service: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Push rejection via WebSocket
    let _ = state
        .service_connections
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;

    // Terminate active WebSocket connection
    state.service_connections.unregister(&service_id).await;

    (StatusCode::OK, Json(model_to_response(updated))).into_response()
}

/// Deactivate a service (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/services/{id}",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service deactivated", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn deactivate_service(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid service ID"),
    };

    let svc = match Service::find_by_id(service_id)
        .filter(service::Column::TenantId.eq(tenant.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    if let Err(e) = active.update(&state.db).await {
        tracing::error!("Failed to deactivate service: {}", e);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Revoke all non-revoked certificates for this service
    if let Err(e) = ServiceCertificate::update_many()
        .col_expr(
            service_certificate::Column::RevokedAt,
            Expr::value(Some(now)),
        )
        .col_expr(
            service_certificate::Column::RevocationReason,
            Expr::value(Some(RevocationReason::ServiceDeactivated)),
        )
        .filter(service_certificate::Column::ServiceId.eq(service_id))
        .filter(service_certificate::Column::RevokedAt.is_null())
        .exec(&state.db)
        .await
    {
        tracing::error!("Failed to revoke certificates: {}", e);
    }

    if let Err(e) =
        crate::settings_store::bump_revocation_version(&state.db, state.default_tenant_id).await
    {
        tracing::warn!(error = ?e, "failed to bump revocation version counter");
    }
    state.revocation_notify.notify_one();

    // Terminate active WebSocket connection
    state.service_connections.unregister(&service_id).await;

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: "Service deactivated".to_string(),
        }),
    )
        .into_response()
}

/// Merge a pending (source) agent into an existing approved (target) agent.
///
/// This operation is only valid for agent services. MQTT services cannot be merged.
#[utoipa::path(
    post,
    path = "/api/v1/services/{target_id}/merge",
    params(
        ("target_id" = String, Path, description = "Target service UUID (approved agent)")
    ),
    request_body = MergeAgentRequest,
    responses(
        (status = 200, description = "Services merged", body = ServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn merge_service(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(target_id): Path<String>,
    Json(body): Json<MergeAgentRequest>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let target_uuid = match uuid::Uuid::parse_str(&target_id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid target service ID"),
    };

    let source_uuid = match uuid::Uuid::parse_str(&body.source_id) {
        Ok(id) => id,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid source service ID"),
    };

    if target_uuid == source_uuid {
        return error_response(StatusCode::BAD_REQUEST, "Cannot merge service into itself");
    }

    // Find target service (must be approved, not deactivated, agent type)
    let target = match Service::find_by_id(target_uuid)
        .filter(service::Column::TenantId.eq(tenant.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Target service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if target.service_type != service::ServiceType::Agent {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Merge is only supported for agent services",
        );
    }

    if target.status != service::ServiceStatus::Approved {
        return error_response(StatusCode::BAD_REQUEST, "Target service must be approved");
    }

    if state.service_connections.is_connected(&target_uuid).await {
        return error_response(
            StatusCode::CONFLICT,
            "Target service is currently connected",
        );
    }

    // Find source service (must be pending, not deactivated, agent type)
    let source = match Service::find_by_id(source_uuid)
        .filter(service::Column::TenantId.eq(tenant.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Source service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if source.service_type != service::ServiceType::Agent {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Merge is only supported for agent services",
        );
    }

    if source.status != service::ServiceStatus::Pending {
        return error_response(StatusCode::BAD_REQUEST, "Source service must be pending");
    }

    let now = OffsetDateTime::now_utc();

    // Save the hash before deactivating the source
    let source_secret_hash = source.enrollment_secret_hash.clone();
    let source_hostname = source.hostname.clone();
    let source_friendly_name = source.friendly_name.clone();
    let source_ip_address = source.ip_address.clone();

    // Deactivate source first -- invalidate its hash to free the unique constraint
    let invalidated_hash = token::hash_token(&token::generate_uuid().to_string());
    let mut source_active: service::ActiveModel = source.into();
    source_active.enrollment_secret_hash = Set(invalidated_hash);
    source_active.deactivated_at = Set(Some(now));
    source_active.updated_at = Set(now);

    if let Err(e) = source_active.update(&state.db).await {
        tracing::error!("Failed to deactivate source service: {}", e);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Revoke all non-revoked certificates for both services
    for (svc_uuid, label) in [(source_uuid, "source"), (target_uuid, "target")] {
        if let Err(e) = ServiceCertificate::update_many()
            .col_expr(
                service_certificate::Column::RevokedAt,
                Expr::value(Some(now)),
            )
            .col_expr(
                service_certificate::Column::RevocationReason,
                Expr::value(Some(RevocationReason::ServiceMerged)),
            )
            .filter(service_certificate::Column::ServiceId.eq(svc_uuid))
            .filter(service_certificate::Column::RevokedAt.is_null())
            .exec(&state.db)
            .await
        {
            tracing::error!("Failed to revoke {label} service certificates: {}", e);
        }
    }

    if let Err(e) =
        crate::settings_store::bump_revocation_version(&state.db, state.default_tenant_id).await
    {
        tracing::warn!(error = ?e, "failed to bump revocation version counter");
    }
    state.revocation_notify.notify_one();

    // Copy source's enrollment_secret_hash to target
    let mut target_active: service::ActiveModel = target.into();
    target_active.enrollment_secret_hash = Set(source_secret_hash);
    target_active.hostname = Set(source_hostname);
    target_active.friendly_name = Set(source_friendly_name);
    target_active.ip_address = Set(source_ip_address);
    target_active.updated_at = Set(now);

    let updated_target = match target_active.update(&state.db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to update target service: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Copy source service's host links to target (INSERT ON CONFLICT DO NOTHING)
    if let Ok(source_links) = ServiceHost::find()
        .filter(service_host::Column::ServiceId.eq(source_uuid))
        .all(&state.db)
        .await
    {
        for link in source_links {
            let existing = ServiceHost::find_by_id((target_uuid, link.host_id))
                .one(&state.db)
                .await;
            if matches!(existing, Ok(None)) {
                let new_link = service_host::ActiveModel {
                    service_id: Set(target_uuid),
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
    state.service_connections.unregister(&source_uuid).await;

    tracing::info!(
        target_id = %target_uuid,
        source_id = %source_uuid,
        "services merged: source deactivated, target updated"
    );

    (StatusCode::OK, Json(model_to_response(updated_target))).into_response()
}

/// Generate a new enrollment token
#[utoipa::path(
    post,
    path = "/api/v1/services/enrollment-token",
    params(
        ("type" = Option<String>, Query, description = "Service type (agent, mqtt). Defaults to agent.")
    ),
    responses(
        (status = 201, description = "Enrollment token generated", body = EnrollmentTokenResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn create_enrollment_token(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let setting_key = match enrollment_setting_key(query.r#type.as_deref()) {
        Ok(k) => k,
        Err(e) => return e.into_response(),
    };

    let plaintext = match token::generate_secure_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to generate enrollment token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hash = match password::hash_password(&plaintext) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to hash enrollment token: {:?}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = upsert_setting(
        &state.db,
        state.default_tenant_id,
        setting_key,
        serde_json::Value::String(hash),
    )
    .await
    {
        tracing::error!("Failed to store enrollment token hash: {:?}", e);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
    path = "/api/v1/services/enrollment-token",
    params(
        ("type" = Option<String>, Query, description = "Service type (agent, mqtt). Defaults to agent.")
    ),
    responses(
        (status = 200, description = "Enrollment token revoked", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn revoke_enrollment_token(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let setting_key = match enrollment_setting_key(query.r#type.as_deref()) {
        Ok(k) => k,
        Err(e) => return e.into_response(),
    };

    if let Err(e) = delete_setting(&state.db, state.default_tenant_id, setting_key).await {
        tracing::error!("Failed to delete enrollment token: {:?}", e);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
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
    path = "/api/v1/services/enrollment-token/status",
    params(
        ("type" = Option<String>, Query, description = "Service type (agent, mqtt). Defaults to agent.")
    ),
    responses(
        (status = 200, description = "Enrollment token status", body = EnrollmentTokenStatusResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn enrollment_token_status(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    if !user.has_permission(Permission::ManageAgents) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let setting_key = match enrollment_setting_key(query.r#type.as_deref()) {
        Ok(k) => k,
        Err(e) => return e.into_response(),
    };

    let configured = matches!(
        load_setting(&state.db, state.default_tenant_id, setting_key).await,
        Ok(Some(_))
    );

    (
        StatusCode::OK,
        Json(EnrollmentTokenStatusResponse { configured }),
    )
        .into_response()
}
