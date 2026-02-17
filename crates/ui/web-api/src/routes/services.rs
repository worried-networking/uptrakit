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
    QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, OnConflict},
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
    // DB and API types are now the same canonical type from shared-types.
    ServiceResponse {
        id: m.id,
        service_type: m.service_type,
        hostname: m.hostname,
        friendly_name: m.friendly_name,
        ip_address: m.ip_address,
        status: m.status,
        client_version: m.client_version,
        last_seen_at: m.last_seen_at.map(format_rfc3339),
        created_at: format_rfc3339(m.created_at),
        updated_at: format_rfc3339(m.updated_at),
    }
}

/// Error returned when the `type` query parameter is invalid.
#[derive(Debug, thiserror::Error)]
#[error("invalid type parameter: '{0}', expected 'agent', 'mqtt', or 'ssh_agent'")]
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
        Some("ssh_agent") => Ok(SettingKey::SshAgentEnrollmentTokenHash),
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
        && let Ok(db_type) = type_filter.parse::<ServiceType>()
    {
        q = q.filter(service::Column::ServiceType.eq(db_type));
    }

    if let Some(ref status_filter) = query.status
        && let Ok(db_status) = status_filter.parse::<ServiceStatus>()
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

/// Get a single service by ID
#[utoipa::path(
    get,
    path = "/api/v1/services/{id}",
    params(
        ("id" = String, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service details", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    security(("bearer_token" = []))
)]
pub async fn get_service(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ViewAgents) {
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

    (StatusCode::OK, Json(model_to_response(svc))).into_response()
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

    // Push approval via WebSocket (local + cross-controller outbox)
    let _ = state
        .notification_service
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

    // Push rejection via WebSocket (local + cross-controller outbox)
    let _ = state
        .notification_service
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

    let source_uuid = body.source_id;

    if target_uuid == source_uuid {
        return error_response(StatusCode::BAD_REQUEST, "Cannot merge service into itself");
    }

    let txn = match state.db.begin().await {
        Ok(txn) => txn,
        Err(e) => {
            tracing::error!("Failed to begin transaction: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Find target service (must be approved, not deactivated, agent type)
    let target = match Service::find_by_id(target_uuid)
        .lock_exclusive()
        .filter(service::Column::TenantId.eq(tenant.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&txn)
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
        .lock_exclusive()
        .filter(service::Column::TenantId.eq(tenant.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&txn)
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

    if let Err(e) = source_active.update(&txn).await {
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
            .exec(&txn)
            .await
        {
            tracing::error!("Failed to revoke {label} service certificates: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    if let Err(e) =
        crate::settings_store::bump_revocation_version(&txn, state.default_tenant_id).await
    {
        tracing::error!(error = ?e, "failed to bump revocation version counter");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    // Copy source's enrollment_secret_hash to target
    let mut target_active: service::ActiveModel = target.into();
    target_active.enrollment_secret_hash = Set(source_secret_hash);
    target_active.hostname = Set(source_hostname);
    target_active.friendly_name = Set(source_friendly_name);
    target_active.ip_address = Set(source_ip_address);
    target_active.updated_at = Set(now);

    let updated_target = match target_active.update(&txn).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to update target service: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Copy source service's host links to target (INSERT ON CONFLICT DO NOTHING)
    let source_links = match ServiceHost::find()
        .filter(service_host::Column::ServiceId.eq(source_uuid))
        .all(&txn)
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::error!("Failed to load source host links: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    for link in source_links {
        let new_link = service_host::ActiveModel {
            service_id: Set(target_uuid),
            host_id: Set(link.host_id),
            linked_at: Set(now),
        };
        if let Err(e) = ServiceHost::insert(new_link)
            .on_conflict(
                OnConflict::columns([
                    service_host::Column::ServiceId,
                    service_host::Column::HostId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(&txn)
            .await
        {
            tracing::error!("Failed to copy host link during merge: {}", e);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    if let Err(e) = txn.commit().await {
        tracing::error!("Failed to commit merge transaction: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    state.revocation_notify.notify_one();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
    use crate::settings::Settings;
    use axum::Json;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use uptrakit_shared_db::entity::prelude::AuthMethod;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn setup_test_db() -> DatabaseConnection {
        let db = test_db().await;

        db.execute_unprepared(
            "CREATE TABLE services (
                id TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                service_type TEXT NOT NULL,
                hostname TEXT NOT NULL,
                friendly_name TEXT NOT NULL,
                ip_address TEXT,
                status TEXT NOT NULL,
                enrollment_secret_hash TEXT NOT NULL UNIQUE,
                client_version TEXT,
                last_seen_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deactivated_at INTEGER
            )",
        )
        .await
        .unwrap();

        db.execute_unprepared(
            "CREATE TABLE service_hosts (
                service_id TEXT NOT NULL,
                host_id TEXT NOT NULL,
                linked_at INTEGER NOT NULL,
                PRIMARY KEY (service_id, host_id)
            )",
        )
        .await
        .unwrap();

        db.execute_unprepared(
            "CREATE TABLE settings_version (
                tenant_id TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                global_version INTEGER NOT NULL,
                revocation_version INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .await
        .unwrap();

        db
    }

    async fn test_state(db: DatabaseConnection, tenant_id: uuid::Uuid) -> Arc<AppState> {
        struct NoopCertSigner;
        #[async_trait::async_trait]
        impl AgentCertSigner for NoopCertSigner {
            async fn sign_agent_csr(
                &self,
                _: &str,
                _: &uuid::Uuid,
                _: time::Duration,
            ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>>
            {
                Err(rootcause::Report::new(CertSignerError::Signing(
                    "noop signer".to_string(),
                )))
            }
            fn active_ca_fingerprint(&self) -> String {
                "0".repeat(64)
            }
        }

        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef =
            Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            }));

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let notification_service = crate::notification_service::NotificationService::new(
            db.clone(),
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        Arc::new(AppState {
            ca_snapshot: ca_rx,
            ca_key_store,
            oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
            account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
            oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                db.clone(),
            ),
            oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                db.clone(),
            ),
            device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
            rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
            db,
            default_tenant_id: tenant_id,
            settings: Settings::new(
                RegistrationSettings {
                    mode: RegistrationMode::Open,
                    token_hash: None,
                    require_token_for_oidc: false,
                },
                7,
            ),
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
            jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                b"test-secret-for-service-merge-tests",
            )),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
            ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            controller_id: uuid::Uuid::nil(),
            notification_service,
            token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
        })
    }

    #[tokio::test]
    async fn merge_service_rolls_back_on_failure() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        let state = test_state(db.clone(), tenant_id).await;

        let now = OffsetDateTime::now_utc();
        let target = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            service_type: Set(service::ServiceType::Agent),
            hostname: Set("target-host".to_string()),
            friendly_name: Set("Target".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set("target-hash".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        let target = target.insert(&db).await.unwrap();

        let source = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            service_type: Set(service::ServiceType::Agent),
            hostname: Set("source-host".to_string()),
            friendly_name: Set("Source".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Pending),
            enrollment_secret_hash: Set("source-hash".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        let source = source.insert(&db).await.unwrap();

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::ManageAgents],
        };

        let response = merge_service(
            State(state),
            TenantContext { tenant_id },
            axum::Extension(auth_user),
            Path(target.id.to_string()),
            Json(MergeAgentRequest {
                source_id: source.id,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let source_after = Service::find_by_id(source.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(source_after.deactivated_at.is_none());
        assert_eq!(source_after.enrollment_secret_hash, "source-hash");
    }
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
