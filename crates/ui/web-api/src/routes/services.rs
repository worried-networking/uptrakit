use crate::AppState;
use crate::auth::permissions::Permission;
use crate::error_response::error_response;
use crate::middleware::permission::{
    CanApproveServices, CanRejectServices, CanRemoveServices, CanUpdateServices, CanViewServices,
};
use crate::middleware::require_auth::AuthenticatedUser;
use crate::queries::services::{self as svc_queries, ServiceQueryError};
use crate::tenant_db::TenantDb;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_internal_wire::{
    ApprovedPayload, ControllerMessage, RejectedPayload, RequestCrlRenewalPayload,
    SetUpdateFreezePayload,
};
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::validation::Validate;
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;
pub use uptrakit_web_api_types::services::{
    ListServicesQuery, MergeAgentRequest, MessageResponse, ServiceResponse, ServiceStatus,
    SetUpdateFreezeRequest, UpdateServiceRequest,
};

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List all services (agents and/or MQTT)
#[utoipa::path(
    get,
    path = "/api/v1/services",
    params(
        ("capability" = Option<String>, Query, description = "Filter by capability (software_discovery, update_tracking, ssh_remote, scheduler)"),
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
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_services(
    tenant_db: TenantDb,
    CanViewServices(_user): CanViewServices,
    Query(query): Query<ListServicesQuery>,
) -> Response {
    match svc_queries::list_services(&tenant_db, &query).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
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
    extensions(("x-required-permission" = json!("view_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_service(
    tenant_db: TenantDb,
    CanViewServices(_user): CanViewServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    match svc_queries::get_active_service(&tenant_db, service_id).await {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
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
    extensions(("x-required-permission" = json!("update_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_service(
    tenant_db: TenantDb,
    CanUpdateServices(_user): CanUpdateServices,
    Path(service_id): Path<Uuid>,
    Json(body): Json<UpdateServiceRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    match svc_queries::update_service_settings(
        &tenant_db,
        service_id,
        body.ping_interval_seconds,
        body.cert_lifetime_hours,
    )
    .await
    {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(e) => {
            tracing::error!("Failed to update service: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Approve a pending service
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/approve",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service approved", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("approve_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn approve_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanApproveServices(_user): CanApproveServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    let resp = match svc_queries::approve_service(&tenant_db, service_id).await {
        Ok(r) => r,
        Err(report) => {
            return match report.current_context() {
                ServiceQueryError::NotFound => {
                    error_response(StatusCode::NOT_FOUND, "Service not found")
                }
                ServiceQueryError::NotPending => {
                    error_response(StatusCode::BAD_REQUEST, "Service is not in pending status")
                }
                ServiceQueryError::Db(_) => {
                    tracing::error!("Failed to approve service: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
                _ => {
                    tracing::error!("Unexpected error approving service: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
            };
        }
    };

    // Push approval via WebSocket (local + cross-controller outbox).
    let _ = state
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    // Dispatch notification event for service enrollment.
    {
        let service_label = resp.service_label.clone();
        state
            .notification_dispatcher
            .dispatch(crate::notifications::events::NotificationEvent {
                tenant_id: tenant_db.tenant_id,
                host_id: None,
                host_name: None,
                software_item_id: None,
                software_item_name: None,
                plugin_type: None,
                details:
                    crate::notifications::events::NotificationEventDetails::NewServiceEnrolled {
                        service_id,
                        service_label,
                    },
            });
    }

    state
        .broadcast
        .event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "approved".to_string(),
            },
        )
        .await;

    (StatusCode::OK, Json(resp)).into_response()
}

/// Reject a pending service
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/reject",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    responses(
        (status = 200, description = "Service rejected", body = ServiceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("reject_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn reject_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanRejectServices(_user): CanRejectServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    let resp = match svc_queries::reject_service(&tenant_db, service_id).await {
        Ok(r) => r,
        Err(report) => {
            return match report.current_context() {
                ServiceQueryError::NotFound => {
                    error_response(StatusCode::NOT_FOUND, "Service not found")
                }
                ServiceQueryError::NotPending => {
                    error_response(StatusCode::BAD_REQUEST, "Service is not in pending status")
                }
                ServiceQueryError::Db(_) => {
                    tracing::error!("Failed to reject service: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
                _ => {
                    tracing::error!("Unexpected error rejecting service: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
            };
        }
    };

    // Push rejection via WebSocket (local + cross-controller outbox).
    let _ = state
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;

    // Terminate active WebSocket connection.
    state
        .service_connections
        .force_disconnect(&service_id)
        .await;

    state
        .broadcast
        .event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "rejected".to_string(),
            },
        )
        .await;

    (StatusCode::OK, Json(resp)).into_response()
}

/// Deactivate a service (soft-delete)
#[utoipa::path(
    delete,
    path = "/api/v1/services/{id}",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    responses(
        (status = 204, description = "Service deactivated"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("remove_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn deactivate_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanRemoveServices(_user): CanRemoveServices,
    Path(service_id): Path<Uuid>,
) -> Response {
    match svc_queries::deactivate_service(&tenant_db, service_id, state.default_tenant_id).await {
        Ok(true) => {
            state.cert.revocation_notify.notify_one();
            state
                .notification_service
                .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                    RequestCrlRenewalPayload::default(),
                ))
                .await;
            state
                .service_connections
                .force_disconnect(&service_id)
                .await;
            state
                .broadcast
                .event_broadcaster
                .send(
                    tenant_db.tenant_id,
                    AdminEvent::ServiceStatusChanged {
                        id: service_id,
                        status: "deactivated".to_string(),
                    },
                )
                .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(report) => match report.current_context() {
            ServiceQueryError::EmbeddedService => error_response(
                StatusCode::CONFLICT,
                "Embedded services cannot be deactivated",
            ),
            _ => {
                tracing::error!("Failed to deactivate service: {}", report);
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        },
    }
}

/// Enable or disable the update freeze on a connected service.
///
/// Sends a `SetUpdateFreeze` wire message to the connected agent. The agent
/// creates or removes the `update-freeze` file in its state directory,
/// immediately blocking or unblocking `ExecuteUpdate` and
/// `ExecuteBatchHostPackageUpdate` processing.
///
/// Returns 404 if the service is not found, 409 if the service is not
/// currently connected, and 200 with a confirmation message on success.
#[utoipa::path(
    post,
    path = "/api/v1/services/{id}/update-freeze",
    params(
        ("id" = Uuid, Path, description = "Service UUID")
    ),
    request_body = SetUpdateFreezeRequest,
    responses(
        (status = 200, description = "Freeze state sent to service", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
        (status = 404, description = "Service not found"),
        (status = 409, description = "Service not connected")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("update_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn set_update_freeze(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateServices(_user): CanUpdateServices,
    Path(service_id): Path<Uuid>,
    Json(body): Json<SetUpdateFreezeRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Verify the service exists in this tenant.
    match svc_queries::get_active_service(&tenant_db, service_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Service not found"),
        Err(report) => {
            tracing::error!("Failed to look up service: {report}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Check that the service is currently connected.
    if !state.service_connections.is_connected(&service_id).await {
        return error_response(StatusCode::CONFLICT, "Service is not currently connected");
    }

    let msg = ControllerMessage::SetUpdateFreeze(SetUpdateFreezePayload {
        enabled: body.enabled,
        reason: body.reason.clone(),
    });

    let sent = state.service_connections.send(&service_id, msg).await;
    if !sent {
        return error_response(StatusCode::CONFLICT, "Service is not currently connected");
    }

    let action = if body.enabled { "enabled" } else { "disabled" };
    tracing::info!(
        %service_id,
        enabled = body.enabled,
        reason = body.reason.as_deref().unwrap_or("-"),
        "security_audit: update freeze {action} for service"
    );

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: format!("Update freeze {action} for service {service_id}."),
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
        ("target_id" = Uuid, Path, description = "Target service UUID (approved agent)")
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
    extensions(("x-required-permission" = json!("update_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn merge_service(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanUpdateServices(_user): CanUpdateServices,
    Path(target_uuid): Path<Uuid>,
    Json(body): Json<MergeAgentRequest>,
) -> Response {
    let source_uuid = body.source_id;

    if target_uuid == source_uuid {
        return error_response(StatusCode::BAD_REQUEST, "Cannot merge service into itself");
    }

    let target_connected = state.service_connections.is_connected(&target_uuid).await;

    let resp = match svc_queries::merge_service(
        &tenant_db,
        target_uuid,
        source_uuid,
        target_connected,
        state.default_tenant_id,
    )
    .await
    {
        Ok(r) => r,
        Err(report) => {
            return match report.current_context() {
                ServiceQueryError::NotFound => {
                    error_response(StatusCode::NOT_FOUND, "Target service not found")
                }
                ServiceQueryError::SourceNotFound => {
                    error_response(StatusCode::NOT_FOUND, "Source service not found")
                }
                ServiceQueryError::TargetConnected => error_response(
                    StatusCode::CONFLICT,
                    "Target service is currently connected",
                ),
                ServiceQueryError::NotMergeable => error_response(
                    StatusCode::BAD_REQUEST,
                    "Merge requires SoftwareDiscovery capability",
                ),
                ServiceQueryError::NotApproved => {
                    error_response(StatusCode::BAD_REQUEST, "Target service must be approved")
                }
                ServiceQueryError::NotPending => {
                    error_response(StatusCode::BAD_REQUEST, "Source service must be pending")
                }
                ServiceQueryError::EmbeddedService => {
                    error_response(StatusCode::CONFLICT, "Embedded services cannot be merged")
                }
                ServiceQueryError::Db(_) => {
                    tracing::error!("Failed to merge services: {}", report);
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                }
            };
        }
    };

    state.cert.revocation_notify.notify_one();
    state
        .notification_service
        .publish_controller_event(ControllerMessage::RequestCrlRenewal(
            RequestCrlRenewalPayload::default(),
        ))
        .await;
    state
        .service_connections
        .force_disconnect(&source_uuid)
        .await;

    tracing::info!(
        target_id = %target_uuid,
        source_id = %source_uuid,
        "services merged: source deactivated, target updated"
    );

    (StatusCode::OK, Json(resp)).into_response()
}

/// Perform a batch action on multiple services.
///
/// Supported actions: `approve`, `reject`, `deactivate`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/services/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Services",
    extensions(("x-required-permission" = json!("approve_services, reject_services, or remove_services"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_services(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    axum::Extension(auth_user): axum::Extension<AuthenticatedUser>,
    Json(body): Json<BatchActionRequest>,
) -> Response {
    if let Err(e) = body.validate() {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let required = match body.action.as_str() {
        "approve" => Permission::ApproveServices,
        "reject" => Permission::RejectServices,
        "deactivate" => Permission::RemoveServices,
        _ => return error_response(StatusCode::BAD_REQUEST, "Unknown batch action"),
    };
    if !auth_user.has_permission(required) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let (succeeded_ids, failed) = match body.action.as_str() {
        "approve" => match svc_queries::batch_approve_services(&tenant_db, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch approve failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "reject" => match svc_queries::batch_reject_services(&tenant_db, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch reject failed: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        "deactivate" => {
            match svc_queries::batch_deactivate_services(
                &tenant_db,
                &body.ids,
                state.default_tenant_id,
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("batch deactivate failed: {e}");
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    );
                }
            }
        }
        unknown => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: approve, reject, deactivate"),
            );
        }
    };

    // Dispatch side effects per succeeded item.
    for id in &succeeded_ids {
        match body.action.as_str() {
            "approve" => {
                let _ = state
                    .notification_service
                    .send(
                        id,
                        ControllerMessage::Approved(ApprovedPayload { service_id: *id }),
                    )
                    .await;
                state
                    .broadcast
                    .event_broadcaster
                    .send(
                        tenant_db.tenant_id,
                        AdminEvent::ServiceStatusChanged {
                            id: *id,
                            status: "approved".to_string(),
                        },
                    )
                    .await;
            }
            "reject" => {
                let _ = state
                    .notification_service
                    .send(
                        id,
                        ControllerMessage::Rejected(RejectedPayload { service_id: *id }),
                    )
                    .await;
                state.service_connections.force_disconnect(id).await;
                state
                    .broadcast
                    .event_broadcaster
                    .send(
                        tenant_db.tenant_id,
                        AdminEvent::ServiceStatusChanged {
                            id: *id,
                            status: "rejected".to_string(),
                        },
                    )
                    .await;
            }
            "deactivate" => {
                state.cert.revocation_notify.notify_one();
                state
                    .notification_service
                    .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                        RequestCrlRenewalPayload::default(),
                    ))
                    .await;
                state.service_connections.force_disconnect(id).await;
                state
                    .broadcast
                    .event_broadcaster
                    .send(
                        tenant_db.tenant_id,
                        AdminEvent::ServiceStatusChanged {
                            id: *id,
                            status: "deactivated".to_string(),
                        },
                    )
                    .await;
            }
            _ => {}
        }
    }

    let response = BatchActionResponse {
        succeeded: succeeded_ids
            .into_iter()
            .map(|id| BatchActionSuccess { id })
            .collect(),
        failed: failed
            .into_iter()
            .map(|(id, error)| BatchActionFailure { id, error })
            .collect(),
    };

    (StatusCode::OK, Json(response)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceCredentialSources;
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
    use crate::middleware::permission::CanUpdateServices;
    use crate::middleware::require_auth::AuthenticatedUser;
    use crate::settings::Settings;
    use crate::tenant_db::TenantDb;
    use axum::Json;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{prelude::Service, service, tenant};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    async fn insert_tenant(db: &DatabaseConnection, id: uuid::Uuid) {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set("Test Tenant".to_string()),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
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
                Err(rootcause::report!(CertSignerError::Signing(
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
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        let settings = Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );

        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            )
            .expect("default catalog should build"),
        );

        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        Arc::new(AppState {
            db: db.clone(),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState {
                jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-secret-for-service-merge-tests",
                )),
                device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            },
            broadcast: crate::app_state::BroadcastState {
                event_broadcaster: crate::event_broadcaster::EventBroadcaster::new(),
                device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster::new(
                ),
                update_output_broadcaster:
                    crate::update_output_broadcaster::UpdateOutputBroadcaster::new(),
                batch_progress_broadcaster:
                    crate::batch_progress_broadcaster::BatchProgressBroadcaster::new(),
            },
            #[cfg(feature = "oidc")]
            oidc: crate::app_state::OidcState {
                oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
                account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
                oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                    db.clone(),
                ),
                oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                    db.clone(),
                ),
            },
            default_tenant_id: tenant_id,
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            controller_id: uuid::Uuid::nil(),
            notification_service,
            notification_dispatcher,
            plugin_ops,
            credential_sources: ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::NoopBackend,
            )),
            extension_registry: Arc::new(crate::extension_registry::ExtensionRegistry::new(vec![])),
            extension_proxy: Arc::new(crate::extension_proxy::ExtensionProxy::new()),
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            reject_dangerous_commands: false,
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
        })
    }

    fn agent_caps_json() -> String {
        use std::collections::BTreeSet;
        use uptrakit_internal_wire::Capability;
        uptrakit_internal_wire::service_profile::serialize_capabilities(&BTreeSet::from([
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
        ]))
    }

    /// Helper: insert a pair of test services (approved target + pending source).
    async fn insert_target_and_source(
        db: &DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) -> (service::Model, service::Model) {
        let now = OffsetDateTime::now_utc();
        let target = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            capabilities: Set(agent_caps_json()),
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
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        };
        let target = target.insert(db).await.unwrap();

        let source = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            capabilities: Set(agent_caps_json()),
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
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        };
        let source = source.insert(db).await.unwrap();

        (target, source)
    }

    /// When the target agent is currently connected the merge must be rejected
    /// with 409 CONFLICT and leave the source service completely unmodified.
    #[tokio::test]
    async fn merge_service_target_connected_returns_conflict() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, source) = insert_target_and_source(&db, tenant_id).await;

        // Register the target as connected — merge must be rejected before any DB changes.
        let caps = {
            use std::collections::BTreeSet;
            use uptrakit_internal_wire::Capability;
            BTreeSet::from([
                Capability::GracefulShutdown,
                Capability::SoftwareDiscovery,
                Capability::UpdateHooks,
            ])
        };
        let (_rx, _token) = state
            .service_connections
            .register(target.id, caps, None, None, None)
            .await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateServices],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = merge_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateServices::new(auth_user),
            Path(target.id),
            Json(MergeAgentRequest {
                source_id: source.id,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);

        // Source must not have been touched.
        let source_after = Service::find_by_id(source.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(source_after.deactivated_at.is_none());
        assert_eq!(source_after.enrollment_secret_hash, "source-hash");
    }

    /// A merge of a valid pending source into an approved target must succeed:
    /// the source is deactivated (with its hash invalidated) and the target
    /// adopts the source's identity fields.
    #[tokio::test]
    async fn merge_service_succeeds_and_deactivates_source() {
        let db = setup_test_db().await;
        let tenant_id = uuid::Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        let state = test_state(db.clone(), tenant_id).await;
        let (target, source) = insert_target_and_source(&db, tenant_id).await;

        let auth_user = AuthenticatedUser {
            user_id: uuid::Uuid::now_v7(),
            auth_method: AuthMethod::Password,
            permissions: vec![Permission::UpdateServices],
        };
        let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);

        let response = merge_service(
            State(Arc::clone(&state)),
            tenant_db,
            CanUpdateServices::new(auth_user),
            Path(target.id),
            Json(MergeAgentRequest {
                source_id: source.id,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        // Source must be deactivated and its original hash must be invalidated.
        let source_after = Service::find_by_id(source.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(
            source_after.deactivated_at.is_some(),
            "source must be deactivated after merge"
        );
        assert_ne!(
            source_after.enrollment_secret_hash, "source-hash",
            "source hash must be invalidated after merge"
        );

        // Target must have adopted the source's identity.
        let target_after = Service::find_by_id(target.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(target_after.hostname, "source-host");
        assert_eq!(target_after.enrollment_secret_hash, "source-hash");
    }
}
