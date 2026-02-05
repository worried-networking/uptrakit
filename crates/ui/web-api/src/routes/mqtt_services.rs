use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::middleware::tenant_context::TenantContext;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_internal_wire::{ApprovedPayload, ControllerMessage, RejectedPayload};
use uptrakit_shared_db::entity::{prelude::Service as MqttService, service as mqtt_service};

pub use uptrakit_web_api_types::agents::MessageResponse;
pub use uptrakit_web_api_types::mqtt_services::{
    ListMqttServicesQuery, MqttServiceResponse, MqttServiceStatus,
};

/// List all MQTT services
#[utoipa::path(
    get,
    path = "/api/v1/mqtt-services",
    params(
        ("status" = Option<String>, Query, description = "Filter by status (pending, approved, rejected, deactivated)")
    ),
    responses(
        (status = 200, description = "List of MQTT services", body = Vec<MqttServiceResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "MQTT Services",
    security(("bearer_token" = []))
)]
pub async fn list_mqtt_services(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<ListMqttServicesQuery>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let mut q = MqttService::find()
        .filter(mqtt_service::Column::TenantId.eq(tenant.tenant_id))
        .filter(mqtt_service::Column::ServiceType.eq(mqtt_service::ServiceType::Mqtt))
        .filter(mqtt_service::Column::DeactivatedAt.is_null());

    if let Some(ref status) = query.status {
        let db_status = match status.as_str() {
            "pending" => Some(mqtt_service::ServiceStatus::Pending),
            "approved" => Some(mqtt_service::ServiceStatus::Approved),
            "rejected" => Some(mqtt_service::ServiceStatus::Rejected),
            "deactivated" => Some(mqtt_service::ServiceStatus::Deactivated),
            _ => None,
        };
        if let Some(s) = db_status {
            q = q.filter(mqtt_service::Column::Status.eq(s));
        }
    }

    let services = match q
        .order_by_desc(mqtt_service::Column::CreatedAt)
        .all(&state.db)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to list MQTT services: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let response: Vec<MqttServiceResponse> =
        services.into_iter().map(service_to_response).collect();
    (StatusCode::OK, Json(response)).into_response()
}

/// Approve a pending MQTT service
#[utoipa::path(
    post,
    path = "/api/v1/mqtt-services/{id}/approve",
    params(
        ("id" = String, Path, description = "MQTT Service UUID")
    ),
    responses(
        (status = 200, description = "MQTT service approved", body = MqttServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "MQTT service not found")
    ),
    tag = "MQTT Services",
    security(("bearer_token" = []))
)]
pub async fn approve_mqtt_service(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid MQTT service ID").into_response(),
    };

    let service = match MqttService::find_by_id(service_id)
        .filter(mqtt_service::Column::TenantId.eq(tenant.tenant_id))
        .filter(mqtt_service::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "MQTT service not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if service.status != mqtt_service::ServiceStatus::Pending {
        return (
            StatusCode::BAD_REQUEST,
            "MQTT service is not in pending status",
        )
            .into_response();
    }

    let now = OffsetDateTime::now_utc();
    let mut active: mqtt_service::ActiveModel = service.into();
    active.status = Set(mqtt_service::ServiceStatus::Approved);
    active.updated_at = Set(now);

    let updated = match active.update(&state.db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to approve MQTT service: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Push approval to connected MQTT service via WebSocket
    let _ = state
        .service_connections
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload {
                service_id: service_id.to_string(),
            }),
        )
        .await;

    (StatusCode::OK, Json(service_to_response(updated))).into_response()
}

/// Reject a pending MQTT service
#[utoipa::path(
    post,
    path = "/api/v1/mqtt-services/{id}/reject",
    params(
        ("id" = String, Path, description = "MQTT Service UUID")
    ),
    responses(
        (status = 200, description = "MQTT service rejected", body = MqttServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "MQTT service not found")
    ),
    tag = "MQTT Services",
    security(("bearer_token" = []))
)]
pub async fn reject_mqtt_service(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid MQTT service ID").into_response(),
    };

    let service = match MqttService::find_by_id(service_id)
        .filter(mqtt_service::Column::TenantId.eq(tenant.tenant_id))
        .filter(mqtt_service::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "MQTT service not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if service.status != mqtt_service::ServiceStatus::Pending {
        return (
            StatusCode::BAD_REQUEST,
            "MQTT service is not in pending status",
        )
            .into_response();
    }

    let now = OffsetDateTime::now_utc();
    let mut active: mqtt_service::ActiveModel = service.into();
    active.status = Set(mqtt_service::ServiceStatus::Rejected);
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    let updated = match active.update(&state.db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to reject MQTT service: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Push rejection to connected MQTT service via WebSocket
    let _ = state
        .service_connections
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload {
                service_id: service_id.to_string(),
            }),
        )
        .await;

    (StatusCode::OK, Json(service_to_response(updated))).into_response()
}

/// Deactivate an MQTT service
#[utoipa::path(
    post,
    path = "/api/v1/mqtt-services/{id}/deactivate",
    params(
        ("id" = String, Path, description = "MQTT Service UUID")
    ),
    responses(
        (status = 200, description = "MQTT service deactivated", body = MessageResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "MQTT service not found")
    ),
    tag = "MQTT Services",
    security(("bearer_token" = []))
)]
pub async fn deactivate_mqtt_service(
    State(state): State<Arc<AppState>>,
    tenant: TenantContext,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> Response {
    if !user.has_permission(Permission::ManageSettings) {
        return (StatusCode::FORBIDDEN, "Insufficient permissions").into_response();
    }

    let service_id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid MQTT service ID").into_response(),
    };

    let service = match MqttService::find_by_id(service_id)
        .filter(mqtt_service::Column::TenantId.eq(tenant.tenant_id))
        .filter(mqtt_service::Column::DeactivatedAt.is_null())
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "MQTT service not found").into_response(),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut active: mqtt_service::ActiveModel = service.into();
    active.status = Set(mqtt_service::ServiceStatus::Deactivated);
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    if let Err(e) = active.update(&state.db).await {
        tracing::error!("Failed to deactivate MQTT service: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Disconnect the service (sends None via channel, closing the WebSocket)
    state.service_connections.unregister(&service_id).await;

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: "MQTT service deactivated".to_string(),
        }),
    )
        .into_response()
}

// --- Helper functions ---

fn format_rfc3339(dt: OffsetDateTime) -> String {
    dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string())
}

fn service_to_response(service: mqtt_service::Model) -> MqttServiceResponse {
    MqttServiceResponse {
        id: service.id.to_string(),
        hostname: service.hostname,
        friendly_name: service.friendly_name,
        status: match service.status {
            mqtt_service::ServiceStatus::Pending => MqttServiceStatus::Pending,
            mqtt_service::ServiceStatus::Approved => MqttServiceStatus::Approved,
            mqtt_service::ServiceStatus::Rejected => MqttServiceStatus::Rejected,
            mqtt_service::ServiceStatus::Deactivated => MqttServiceStatus::Deactivated,
        },
        last_seen_at: service.last_seen_at.map(format_rfc3339),
        created_at: format_rfc3339(service.created_at),
        updated_at: format_rfc3339(service.updated_at),
    }
}
