//! Service CRUD route handlers: list / get / update.

use super::{ListServicesQuery, PaginatedResponse, ServiceResponse, UpdateServiceRequest};
use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::action::{CanReadServices, CanUpdateServices};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::services as svc_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AuditEntry, AuditOutcome, Stateful};
use uptrakit_web_api_queries::queries::services::ServiceView;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

/// List all services (agents and/or MQTT)
#[utoipa::path(
    get,
    path = "/api/v1/services",
    params(ListServicesQuery),
    responses(
        (status = 200, description = "Paginated list of services", body = PaginatedResponse<ServiceResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    security(("oauth2" = ["services:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_services(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanReadServices(_user): CanReadServices,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    match svc_queries::list_services(&tenant_db, &query).await {
        Ok(mut resp) => {
            let trust_domain = state.tls_config_rx.borrow().trust_domain.clone();
            if !trust_domain.is_empty() {
                for svc in &mut resp.items {
                    svc.spiffe_id = Some(format!("spiffe://{trust_domain}/service/{}", svc.id));
                }
            }
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list services: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a single service by ID
#[utoipa::path(
    get,
    path = "/api/v1/services/{id}",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service details", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    security(("oauth2" = ["services:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanReadServices(_user): CanReadServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    match svc_queries::get_active_service_detail(&tenant_db, service_id).await {
        Ok(Some(mut resp)) => {
            let trust_domain = state.tls_config_rx.borrow().trust_domain.clone();
            if !trust_domain.is_empty() {
                resp.spiffe_id = Some(format!("spiffe://{trust_domain}/service/{}", resp.id));
            }
            (StatusCode::OK, Json(resp)).into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("DB error: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a service's configurable settings (e.g. ping interval)
#[utoipa::path(
    put,
    path = "/api/v1/services/{id}",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    request_body = UpdateServiceRequest,
    responses(
        (status = 200, description = "Service updated", body = ServiceResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    security(("oauth2" = ["services:update"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateServices(user): CanUpdateServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
    Json(body): Json<UpdateServiceRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();
    let trust_domain = state.tls_config_rx.borrow().trust_domain.clone();

    if let Err(e) = body.validate() {
        // ValidationFailed — fire-and-forget (no DB mutation happened).
        let entry = uptrakit_audit_log::AuditEntry::builder(
            uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("service", service_id.to_string(), None)
        .outcome(uptrakit_audit_log::AuditOutcome::ValidationFailed)
        .details(serde_json::json!({ "reason_code": "invalid_request" }))
        .build();
        if let Ok(entry) = entry {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for service update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let result = svc_queries::update_service_settings_in_tx(
        &tx,
        tenant_id,
        service_id,
        body.ping_interval_seconds,
        body.cert_lifetime_hours,
    )
    .await;

    match result {
        Ok(Some((before, after))) => {
            let before_view = ServiceView::from(&before);
            let after_view = ServiceView::from(&after);
            let hook = state.audit_emitter.commit_hook();
            let audit_entry =
                match AuditEntry::<Stateful>::service_update(&before_view, &after_view)
                    .tenant_scope(tenant_id)
                    .actor(actor_type, actor_id)
                    .outcome(AuditOutcome::Success)
                    .details(serde_json::json!({
                        "ping_interval_seconds": body.ping_interval_seconds,
                        "cert_lifetime_hours": body.cert_lifetime_hours,
                    }))
                    .build()
                {
                    Ok(entry) => entry,
                    Err(e) => {
                        tracing::error!("Failed to build audit entry for service update: {e}");
                        drop(tx);
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                };
            if let Err(e) = state
                .audit_emitter
                .emit_stateful(&tx, &hook, audit_entry)
                .await
            {
                tracing::error!("Failed to emit stateful audit for service update: {e}");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            if let Err(e) = tx.commit().await {
                tracing::error!("Failed to commit service update: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
            hook.flush_after_commit().await;

            match svc_queries::get_active_service(&tenant_db, service_id).await {
                Ok(Some(mut resp)) => {
                    if !trust_domain.is_empty() {
                        resp.spiffe_id =
                            Some(format!("spiffe://{trust_domain}/service/{}", resp.id));
                    }
                    (StatusCode::OK, Json(resp)).into_response()
                }
                Ok(None) => {
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
                Err(e) => {
                    tracing::error!("Failed to fetch updated service: {e}");
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
            }
        }
        Ok(None) => {
            drop(tx);
            // Denied — fire-and-forget (no DB mutation happened).
            let entry = uptrakit_audit_log::AuditEntry::builder(
                uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("service", service_id.to_string(), None)
            .outcome(uptrakit_audit_log::AuditOutcome::Denied)
            .details(serde_json::json!({ "reason_code": "service.not_found" }))
            .build();
            if let Ok(entry) = entry {
                state.audit_emitter.emit_event(entry);
            }
            error_response(StatusCode::NOT_FOUND, "Service not found")
        }
        Err(e) => {
            tracing::error!("Failed to update service: {}", e);
            drop(tx);
            // Failed — fire-and-forget.
            let entry = uptrakit_audit_log::AuditEntry::builder(
                uptrakit_audit_log::AuditActionType::SERVICE_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("service", service_id.to_string(), None)
            .outcome(uptrakit_audit_log::AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "service.database_error" }))
            .build();
            if let Ok(entry) = entry {
                state.audit_emitter.emit_event(entry);
            }
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
