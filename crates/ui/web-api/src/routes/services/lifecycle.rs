//! Service status-lifecycle route handlers: approve / reject / deactivate / set-update-freeze.

use super::audit::{AuditContext, emit_service_lifecycle_audit};
use super::{MessageResponse, ServiceResponse, SetUpdateFreezeRequest};
use crate::AppState;
use crate::api_error::ApiError;
use crate::error_response::error_response;
use crate::middleware::permission::{
    CanApproveServices, CanRejectServices, CanRemoveServices, CanUpdateServices,
};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
use crate::queries::services as svc_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
use std::sync::Arc;
use uptrakit_audit_log::{AuditEntry, AuditOutcome, Stateful};
use uptrakit_web_api_queries::queries::services::ServiceView;
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::validation::Validate;
use uptrakit_wire::{
    ApprovedPayload, ControllerMessage, RejectedPayload, RequestCrlRenewalPayload,
    SetUpdateFreezePayload,
};
use uuid::Uuid;

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
    CanApproveServices(user): CanApproveServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

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
            tracing::error!("Failed to begin transaction for service approve: {e}");
            return Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "internal_error",
                None,
            ));
        }
    };

    let (before, after) = match svc_queries::approve_service_in_tx(&tx, tenant_id, service_id).await
    {
        Ok(pair) => pair,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            drop(tx);
            let entry = uptrakit_audit_log::AuditEntry::builder(
                uptrakit_audit_log::AuditActionType::SERVICE_APPROVE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("service", service_id.to_string(), None)
            .outcome(outcome)
            .details(serde_json::json!({ "reason_code": reason_code }))
            .build();
            if let Ok(entry) = entry {
                state.audit_emitter.emit_event(entry);
            }
            return Err(err.into());
        }
    };

    let before_view = ServiceView::from(&before);
    let after_view = ServiceView::from(&after);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::service_approve(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({}))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for service approve: {e}");
            drop(tx);
            return Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "internal_error",
                None,
            ));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit stateful audit for service approve: {e}");
        drop(tx);
        return Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "An internal error occurred.",
            "internal_error",
            None,
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit service approve: {e}");
        return Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "An internal error occurred.",
            "internal_error",
            None,
        ));
    }
    hook.flush_after_commit().await;

    // Side effects (after commit).
    let _ = state
        .notification
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    let service_label = {
        use uptrakit_wire::Capability;
        use uptrakit_wire::service_profile::{ServiceProfile, parse_capabilities};
        let caps = parse_capabilities(&after.capabilities);
        let has_ssh = caps.contains(&Capability::SshRemote);
        ServiceProfile::from_capabilities(&caps)
            .service_label(has_ssh)
            .to_string()
    };
    state
        .notification
        .notification_dispatcher
        .dispatch(NotificationEvent::new(
            tenant_id,
            NotificationEventDetails::NewServiceEnrolled {
                service_id,
                service_label,
            },
        ));

    state
        .notification
        .event_broadcaster
        .send(
            tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "approved".to_string(),
            },
        )
        .await;

    match svc_queries::get_active_service(&tenant_db, service_id).await {
        Ok(Some(resp)) => Ok((StatusCode::OK, Json(resp)).into_response()),
        Ok(None) => Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "An internal error occurred.",
            "internal_error",
            None,
        )),
        Err(e) => {
            tracing::error!("Failed to fetch approved service: {e}");
            Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "internal_error",
                None,
            ))
        }
    }
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
    CanRejectServices(user): CanRejectServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

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
            tracing::error!("Failed to begin transaction for service reject: {e}");
            return Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "internal_error",
                None,
            ));
        }
    };

    let (before, after) = match svc_queries::reject_service_in_tx(&tx, tenant_id, service_id).await
    {
        Ok(pair) => pair,
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            drop(tx);
            let entry = uptrakit_audit_log::AuditEntry::builder(
                uptrakit_audit_log::AuditActionType::SERVICE_REJECT,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("service", service_id.to_string(), None)
            .outcome(outcome)
            .details(serde_json::json!({ "reason_code": reason_code }))
            .build();
            if let Ok(entry) = entry {
                state.audit_emitter.emit_event(entry);
            }
            return Err(err.into());
        }
    };

    let before_view = ServiceView::from(&before);
    let after_view = ServiceView::from(&after);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::service_reject(&before_view, &after_view)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({}))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for service reject: {e}");
            drop(tx);
            return Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "internal_error",
                None,
            ));
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit stateful audit for service reject: {e}");
        drop(tx);
        return Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "An internal error occurred.",
            "internal_error",
            None,
        ));
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit service reject: {e}");
        return Err(ApiError::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "An internal error occurred.",
            "internal_error",
            None,
        ));
    }
    hook.flush_after_commit().await;

    // Side effects (after commit).
    let _ = state
        .notification
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;
    state
        .service_connections
        .force_disconnect(&service_id)
        .await;
    state
        .notification
        .event_broadcaster
        .send(
            tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "rejected".to_string(),
            },
        )
        .await;

    match svc_queries::get_active_service(&tenant_db, service_id).await {
        Ok(Some(resp)) => Ok((StatusCode::OK, Json(resp)).into_response()),
        // Service was rejected (deactivated_at set), so get_active_service returns None.
        // Build a minimal response from the after model.
        Ok(None) => {
            use uptrakit_web_api_types::services::{ServiceResponse, ServiceStatus as WireStatus};
            use uptrakit_wire::Capability;
            use uptrakit_wire::service_profile::{ServiceProfile, parse_capabilities};
            let caps = parse_capabilities(&after.capabilities);
            let has_ssh = caps.contains(&Capability::SshRemote);
            let profile = ServiceProfile::from_capabilities(&caps);
            let cap_strings: Vec<String> = caps.iter().map(|c| c.as_str().to_string()).collect();
            let resp = ServiceResponse::new(
                after.id,
                cap_strings,
                profile.service_label(has_ssh).to_string(),
                after.hostname.clone(),
                after.friendly_name.clone(),
                after.is_embedded,
                after.ip_address.clone(),
                WireStatus::Rejected,
                after.client_version.clone(),
                after.last_seen_at,
                after.created_at,
                after.updated_at,
                after.ping_interval_seconds.map(|v| v as u32),
                after.cert_lifetime_hours.map(|v| v as u32),
                None, // yielded_to
                None, // spiffe_id
                None, // cert_serial_number
            );
            Ok((StatusCode::OK, Json(resp)).into_response())
        }
        Err(e) => {
            tracing::error!("Failed to fetch rejected service: {e}");
            Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "internal_error",
                None,
            ))
        }
    }
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
    CanRemoveServices(user): CanRemoveServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

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
            tracing::error!("Failed to begin transaction for service deactivate: {e}");
            return Err(ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred.",
                "internal_error",
                None,
            ));
        }
    };

    let result =
        svc_queries::deactivate_service_in_tx(&tx, tenant_id, service_id, state.default_tenant_id)
            .await;

    match result {
        Ok(Some(before)) => {
            let before_view = ServiceView::from(&before);
            // After view: same as before but with deactivated_at set.
            // We build a synthetic after for the snapshot.
            let after_view = {
                let mut v = ServiceView::from(&before);
                v.status = uptrakit_shared_types::ServiceStatus::Deactivated.to_string();
                v
            };
            let hook = state.audit_emitter.commit_hook();
            let audit_entry =
                match AuditEntry::<Stateful>::service_deactivate(&before_view, &after_view)
                    .tenant_scope(tenant_id)
                    .actor(actor_type, actor_id)
                    .outcome(AuditOutcome::Success)
                    .details(serde_json::json!({}))
                    .build()
                {
                    Ok(entry) => entry,
                    Err(e) => {
                        tracing::error!("Failed to build audit entry for service deactivate: {e}");
                        drop(tx);
                        return Err(ApiError::new(
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            "An internal error occurred.",
                            "internal_error",
                            None,
                        ));
                    }
                };

            if let Err(e) = state
                .audit_emitter
                .emit_stateful(&tx, &hook, audit_entry)
                .await
            {
                tracing::error!("Failed to emit stateful audit for service deactivate: {e}");
                drop(tx);
                return Err(ApiError::new(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "An internal error occurred.",
                    "internal_error",
                    None,
                ));
            }

            if let Err(e) = tx.commit().await {
                tracing::error!("Failed to commit service deactivate: {e}");
                return Err(ApiError::new(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "An internal error occurred.",
                    "internal_error",
                    None,
                ));
            }
            hook.flush_after_commit().await;

            // Side effects (after commit).
            state.cert.revocation_notify.notify_one();
            state
                .notification
                .notification_service
                .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                    RequestCrlRenewalPayload::default(),
                ))
                .await;
            state
                .notification
                .event_broadcaster
                .send(
                    tenant_id,
                    AdminEvent::ServiceStatusChanged {
                        id: service_id,
                        status: "deactivated".to_string(),
                    },
                )
                .await;
            state
                .service_connections
                .force_disconnect(&service_id)
                .await;

            Ok(StatusCode::NO_CONTENT.into_response())
        }
        Ok(None) => {
            drop(tx);
            // Not found — fire-and-forget denied audit.
            let entry = uptrakit_audit_log::AuditEntry::builder(
                uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
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
            Ok(error_response(StatusCode::NOT_FOUND, "Service not found"))
        }
        Err(err) => {
            let (outcome, reason_code) = err.current_context().audit_classification();
            drop(tx);
            let entry = uptrakit_audit_log::AuditEntry::builder(
                uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("service", service_id.to_string(), None)
            .outcome(outcome)
            .details(serde_json::json!({ "reason_code": reason_code }))
            .build();
            if let Ok(entry) = entry {
                state.audit_emitter.emit_event(entry);
            }
            Err(err.into())
        }
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
    CanUpdateServices(user): CanUpdateServices,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(service_id): Path<Uuid>,
    Json(body): Json<SetUpdateFreezeRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };
    let action_type = if body.enabled {
        uptrakit_audit_log::AuditActionType::SERVICE_UPDATE_FREEZE_ENABLE
    } else {
        uptrakit_audit_log::AuditActionType::SERVICE_UPDATE_FREEZE_DISABLE
    };
    if let Err(e) = body.validate() {
        emit_service_lifecycle_audit(
            &audit_ctx,
            action_type,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "enabled": body.enabled,
                "reason_present": body.reason.is_some(),
                "reason_code": "invalid_request",
            }),
        );
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    // Verify the service exists in this tenant.
    match svc_queries::get_active_service(&tenant_db, service_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            emit_service_lifecycle_audit(
                &audit_ctx,
                action_type,
                service_id,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "enabled": body.enabled,
                    "reason_present": body.reason.is_some(),
                    "reason_code": "service.not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Service not found");
        }
        Err(report) => {
            tracing::error!("Failed to look up service: {report}");
            emit_service_lifecycle_audit(
                &audit_ctx,
                action_type,
                service_id,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "enabled": body.enabled,
                    "reason_present": body.reason.is_some(),
                    "reason_code": "service.database_error",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    }

    // Check that the service is currently connected.
    if !state.service_connections.is_connected(&service_id).await {
        emit_service_lifecycle_audit(
            &audit_ctx,
            action_type,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "enabled": body.enabled,
                "reason_present": body.reason.is_some(),
                "reason_code": "service.not_connected",
            }),
        );
        return error_response(StatusCode::CONFLICT, "Service is not currently connected");
    }

    let msg = ControllerMessage::SetUpdateFreeze(SetUpdateFreezePayload {
        enabled: body.enabled,
        reason: body.reason.clone(),
    });

    let sent = state.service_connections.send(&service_id, msg).await;
    if !sent {
        emit_service_lifecycle_audit(
            &audit_ctx,
            action_type,
            service_id,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "enabled": body.enabled,
                "reason_present": body.reason.is_some(),
                "reason_code": "service.not_connected",
            }),
        );
        return error_response(StatusCode::CONFLICT, "Service is not currently connected");
    }

    let action = if body.enabled { "enabled" } else { "disabled" };
    emit_service_lifecycle_audit(
        &audit_ctx,
        action_type,
        service_id,
        None,
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "enabled": body.enabled,
            "reason_present": body.reason.is_some(),
        }),
    );

    (
        StatusCode::OK,
        Json(MessageResponse {
            message: format!("Update freeze {action} for service {service_id}."),
        }),
    )
        .into_response()
}
