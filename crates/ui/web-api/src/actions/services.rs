use uuid::Uuid;

use crate::actions::MutationContext;
use crate::app_state::CertState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
use crate::queries::services::{self as svc_queries, ServiceQueryError};
use crate::service_connections::ServiceConnectionRegistry;
use crate::tenant_db::TenantDb;
use uptrakit_internal_wire::{
    ApprovedPayload, ControllerMessage, RejectedPayload, RequestCrlRenewalPayload,
};
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::services::ServiceResponse;

pub(crate) async fn approve(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
) -> Result<ServiceResponse, rootcause::Report<ServiceQueryError>> {
    let resp = svc_queries::approve_service(tenant_db, service_id).await?;

    let _ = ctx
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    let service_label = resp.service_label.clone();
    ctx.notification_dispatcher.dispatch(NotificationEvent {
        tenant_id: tenant_db.tenant_id,
        host_id: None,
        host_name: None,
        software_item_id: None,
        software_item_name: None,
        plugin_type: None,
        details: NotificationEventDetails::NewServiceEnrolled {
            service_id,
            service_label,
        },
    });

    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "approved".to_string(),
            },
        )
        .await;

    Ok(resp)
}

/// NOTE: Does NOT dispatch a `NotificationEvent` (preserved asymmetry — individual approve only).
pub(crate) async fn reject(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
    service_connections: &ServiceConnectionRegistry,
) -> Result<ServiceResponse, rootcause::Report<ServiceQueryError>> {
    let resp = svc_queries::reject_service(tenant_db, service_id).await?;

    let _ = ctx
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;

    service_connections.force_disconnect(&service_id).await;

    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "rejected".to_string(),
            },
        )
        .await;

    Ok(resp)
}

/// Returns `Ok(true)` when the service was found and deactivated; `Ok(false)` when not found.
/// No side effects are run when `Ok(false)`.
pub(crate) async fn deactivate(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
    default_tenant_id: Uuid,
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<bool, rootcause::Report<ServiceQueryError>> {
    let found = svc_queries::deactivate_service(tenant_db, service_id, default_tenant_id).await?;
    if !found {
        return Ok(false);
    }

    cert.revocation_notify.notify_one();
    ctx.notification_service
        .publish_controller_event(ControllerMessage::RequestCrlRenewal(
            RequestCrlRenewalPayload::default(),
        ))
        .await;
    service_connections.force_disconnect(&service_id).await;

    ctx.event_broadcaster
        .send(
            tenant_db.tenant_id,
            AdminEvent::ServiceStatusChanged {
                id: service_id,
                status: "deactivated".to_string(),
            },
        )
        .await;

    Ok(true)
}

/// Parameters for a service merge operation.
pub(crate) struct MergeParams {
    pub target_id: Uuid,
    pub source_id: Uuid,
    pub target_connected: bool,
    pub default_tenant_id: Uuid,
}

/// NOTE: Does NOT broadcast `ServiceStatusChanged` (preserved asymmetry — merge has no status event).
pub(crate) async fn merge(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    params: MergeParams,
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<ServiceResponse, rootcause::Report<ServiceQueryError>> {
    let resp = svc_queries::merge_service(
        tenant_db,
        params.target_id,
        params.source_id,
        params.target_connected,
        params.default_tenant_id,
    )
    .await?;

    cert.revocation_notify.notify_one();
    ctx.notification_service
        .publish_controller_event(ControllerMessage::RequestCrlRenewal(
            RequestCrlRenewalPayload::default(),
        ))
        .await;
    service_connections
        .force_disconnect(&params.source_id)
        .await;

    // Preserved asymmetry: merge does NOT broadcast ServiceStatusChanged.
    // See design doc "Preserved asymmetric behaviors" item 1.

    Ok(resp)
}

/// NOTE: Does NOT dispatch a `NotificationEvent` (preserved asymmetry — individual approve only).
pub(crate) async fn batch_approve(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<ServiceQueryError>> {
    let (succeeded_ids, failed) = svc_queries::batch_approve_services(tenant_db, ids).await?;

    for id in &succeeded_ids {
        let _ = ctx
            .notification_service
            .send(
                id,
                ControllerMessage::Approved(ApprovedPayload { service_id: *id }),
            )
            .await;

        // Preserved asymmetry: batch approve does NOT dispatch NotificationEvent::NewServiceEnrolled.
        // Only individual approve triggers the notification dispatcher.

        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::ServiceStatusChanged {
                    id: *id,
                    status: "approved".to_string(),
                },
            )
            .await;
    }

    Ok((succeeded_ids, failed))
}

pub(crate) async fn batch_reject(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
    service_connections: &ServiceConnectionRegistry,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<ServiceQueryError>> {
    let (succeeded_ids, failed) = svc_queries::batch_reject_services(tenant_db, ids).await?;

    for id in &succeeded_ids {
        let _ = ctx
            .notification_service
            .send(
                id,
                ControllerMessage::Rejected(RejectedPayload { service_id: *id }),
            )
            .await;
        service_connections.force_disconnect(id).await;
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::ServiceStatusChanged {
                    id: *id,
                    status: "rejected".to_string(),
                },
            )
            .await;
    }

    Ok((succeeded_ids, failed))
}

pub(crate) async fn batch_deactivate(
    tenant_db: &TenantDb,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
    default_tenant_id: Uuid,
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<ServiceQueryError>> {
    let (succeeded_ids, failed) =
        svc_queries::batch_deactivate_services(tenant_db, ids, default_tenant_id).await?;

    for id in &succeeded_ids {
        cert.revocation_notify.notify_one();
        ctx.notification_service
            .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                RequestCrlRenewalPayload::default(),
            ))
            .await;
        service_connections.force_disconnect(id).await;
        ctx.event_broadcaster
            .send(
                tenant_db.tenant_id,
                AdminEvent::ServiceStatusChanged {
                    id: *id,
                    status: "deactivated".to_string(),
                },
            )
            .await;
    }

    Ok((succeeded_ids, failed))
}
