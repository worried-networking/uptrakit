use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::actions::MutationContext;
use crate::app_state::CertState;
use crate::queries::system_services::{self as ss_queries, SystemServiceQueryError};
use crate::service_connections::ServiceConnectionRegistry;
use uptrakit_internal_wire::{
    ApprovedPayload, ControllerMessage, RejectedPayload, RequestCrlRenewalPayload,
};
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_web_api_types::system_services::SystemServiceResponse;

pub(crate) async fn approve(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
) -> Result<SystemServiceResponse, rootcause::Report<SystemServiceQueryError>> {
    let resp = ss_queries::approve_system_service(db, service_id).await?;

    let _ = ctx
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Approved(ApprovedPayload { service_id }),
        )
        .await;

    ctx.event_broadcaster
        .send_global(AdminEvent::SystemServiceStatusChanged {
            id: service_id,
            status: "approved".to_string(),
        })
        .await;

    Ok(resp)
}

pub(crate) async fn reject(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
    service_connections: &ServiceConnectionRegistry,
) -> Result<SystemServiceResponse, rootcause::Report<SystemServiceQueryError>> {
    let resp = ss_queries::reject_system_service(db, service_id).await?;

    let _ = ctx
        .notification_service
        .send(
            &service_id,
            ControllerMessage::Rejected(RejectedPayload { service_id }),
        )
        .await;

    service_connections.force_disconnect(&service_id).await;

    ctx.event_broadcaster
        .send_global(AdminEvent::SystemServiceStatusChanged {
            id: service_id,
            status: "rejected".to_string(),
        })
        .await;

    Ok(resp)
}

/// Returns `Ok(true)` when the service was found and deactivated; `Ok(false)` when not found.
/// No side effects are run when `Ok(false)`.
pub(crate) async fn deactivate(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    service_id: Uuid,
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<bool, rootcause::Report<SystemServiceQueryError>> {
    let found = ss_queries::deactivate_system_service(db, service_id).await?;
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
        .send_global(AdminEvent::SystemServiceStatusChanged {
            id: service_id,
            status: "deactivated".to_string(),
        })
        .await;

    Ok(true)
}

pub(crate) async fn batch_approve(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<SystemServiceQueryError>> {
    let (succeeded_ids, failed) = ss_queries::batch_approve_system_services(db, ids).await?;

    for id in &succeeded_ids {
        let _ = ctx
            .notification_service
            .send(
                id,
                ControllerMessage::Approved(ApprovedPayload { service_id: *id }),
            )
            .await;
        ctx.event_broadcaster
            .send_global(AdminEvent::SystemServiceStatusChanged {
                id: *id,
                status: "approved".to_string(),
            })
            .await;
    }

    Ok((succeeded_ids, failed))
}

pub(crate) async fn batch_reject(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
    service_connections: &ServiceConnectionRegistry,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<SystemServiceQueryError>> {
    let (succeeded_ids, failed) = ss_queries::batch_reject_system_services(db, ids).await?;

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
            .send_global(AdminEvent::SystemServiceStatusChanged {
                id: *id,
                status: "rejected".to_string(),
            })
            .await;
    }

    Ok((succeeded_ids, failed))
}

pub(crate) async fn batch_deactivate(
    db: &DatabaseConnection,
    ctx: &MutationContext<'_>,
    ids: &[Uuid],
    cert: &CertState,
    service_connections: &ServiceConnectionRegistry,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), rootcause::Report<SystemServiceQueryError>> {
    let (succeeded_ids, failed) = ss_queries::batch_deactivate_system_services(db, ids).await?;

    for id in &succeeded_ids {
        cert.revocation_notify.notify_one();
        ctx.notification_service
            .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                RequestCrlRenewalPayload::default(),
            ))
            .await;
        service_connections.force_disconnect(id).await;
        ctx.event_broadcaster
            .send_global(AdminEvent::SystemServiceStatusChanged {
                id: *id,
                status: "deactivated".to_string(),
            })
            .await;
    }

    Ok((succeeded_ids, failed))
}
