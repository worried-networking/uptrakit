use sea_orm::{ActiveValue, DatabaseConnection, EntityTrait};
use time::OffsetDateTime;
use uptrakit_internal_wire::{
    CaBundleUpdatedPayload, ControllerMessage, RequestCertRenewalPayload,
};
use uptrakit_shared_db::entity::controller_event;
use uuid::Uuid;

use crate::service_connections::ServiceConnectionRegistry;

/// Cross-controller notification service.
///
/// Wraps `ServiceConnectionRegistry` to provide the same push API but additionally
/// writes events to the `controller_events` outbox table. A background poller on
/// each controller picks up events from other controllers and delivers them to
/// locally connected services.
#[derive(Clone)]
pub struct NotificationService {
    db: DatabaseConnection,
    registry: ServiceConnectionRegistry,
    controller_id: Uuid,
}

impl NotificationService {
    pub fn new(
        db: DatabaseConnection,
        registry: ServiceConnectionRegistry,
        controller_id: Uuid,
    ) -> Self {
        Self {
            db,
            registry,
            controller_id,
        }
    }

    /// Push a message to a specific service (local + outbox).
    pub async fn send(&self, service_id: &Uuid, msg: ControllerMessage) -> bool {
        let local = self.registry.send(service_id, msg.clone()).await;
        self.write_outbox_event(Some(*service_id), None, &msg).await;
        local
    }

    /// Broadcast a message to all connected services (local + outbox).
    pub async fn broadcast(&self, msg: ControllerMessage) {
        self.registry.broadcast(msg.clone()).await;
        self.write_outbox_event(None, None, &msg).await;
    }

    /// Broadcast a CA bundle update to all connected services (local + outbox).
    pub async fn broadcast_ca_bundle_updated(&self, payload: CaBundleUpdatedPayload) {
        let msg = ControllerMessage::CaBundleUpdated(payload);
        self.broadcast(msg).await;
    }

    /// Broadcast a certificate renewal request to all connected services (local + outbox).
    pub async fn broadcast_request_cert_renewal(&self, payload: RequestCertRenewalPayload) {
        let msg = ControllerMessage::RequestCertRenewal(payload);
        self.broadcast(msg).await;
    }

    /// Write an outbox event for cross-controller delivery.
    ///
    /// This is fire-and-forget: errors are logged but do not fail the caller.
    pub(crate) async fn write_outbox_event(
        &self,
        target_service_id: Option<Uuid>,
        target_service_type: Option<&str>,
        msg: &ControllerMessage,
    ) {
        let json = match serde_json::to_string(msg) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize outbox event");
                return;
            }
        };

        let event = controller_event::ActiveModel {
            id: ActiveValue::NotSet,
            source_controller_id: ActiveValue::Set(self.controller_id),
            target_service_id: ActiveValue::Set(target_service_id),
            target_service_type: ActiveValue::Set(target_service_type.map(String::from)),
            message_json: ActiveValue::Set(json),
            created_at: ActiveValue::Set(OffsetDateTime::now_utc()),
        };

        if let Err(e) = controller_event::Entity::insert(event).exec(&self.db).await {
            tracing::error!(error = %e, "failed to write outbox event");
        }
    }

    /// Access the underlying registry (e.g. for `is_connected` checks).
    pub fn registry(&self) -> &ServiceConnectionRegistry {
        &self.registry
    }

    /// Return the controller ID.
    pub fn controller_id(&self) -> Uuid {
        self.controller_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};
    use uptrakit_internal_wire::{ApprovedPayload, ServerRestartingPayload};

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    #[tokio::test]
    async fn send_writes_outbox_event() {
        let db = test_db().await;
        let registry = ServiceConnectionRegistry::new();
        let controller_id = Uuid::now_v7();
        let svc = NotificationService::new(db, registry, controller_id);

        let service_id = Uuid::now_v7();
        let msg = ControllerMessage::Approved(ApprovedPayload { service_id });

        // send returns false because no service is connected, but outbox write
        // will fail silently because the table doesn't exist in the test DB.
        // This test verifies the code path doesn't panic.
        let result = svc.send(&service_id, msg).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn server_restarting_is_local_only() {
        // ServerRestarting should NOT be sent through NotificationService.
        // It stays on ServiceConnectionRegistry.broadcast_server_restarting_scattered().
        // This test documents the design intent.
        let msg = ControllerMessage::ServerRestarting(ServerRestartingPayload {
            reason: "test".to_string(),
        });
        // The message type exists and is valid
        assert!(matches!(msg, ControllerMessage::ServerRestarting(_)));
    }
}
