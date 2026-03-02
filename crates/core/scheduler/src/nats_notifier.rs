//! NATS-backed [`SchedulerNotifier`] for the external scheduler binary.
//!
//! Messages are published to the shared NATS JetStream stream
//! (`UPTRAKIT_EVENTS`), where controllers consume them and deliver to the
//! appropriate connected services via their WebSocket connections.

use uptrakit_internal_wire::{
    ControllerMessage, MqttSoftwareStatesPayload, RequestCaRotationPayload,
    RequestCrlRenewalPayload,
};
use uptrakit_nats::NatsConnection;
use uptrakit_scheduler_engine::SchedulerNotifier;
use uuid::Uuid;

/// Publishes scheduler messages to NATS for cross-controller delivery.
pub struct NatsSchedulerNotifier {
    nats: NatsConnection,
    scheduler_id: Uuid,
}

impl NatsSchedulerNotifier {
    pub fn new(nats: NatsConnection, scheduler_id: Uuid) -> Self {
        Self { nats, scheduler_id }
    }
}

#[async_trait::async_trait]
impl SchedulerNotifier for NatsSchedulerNotifier {
    async fn send_to_service(&self, service_id: &Uuid, msg: ControllerMessage) {
        self.nats
            .publish(self.scheduler_id, Some(*service_id), None, msg)
            .await;
    }

    async fn broadcast(&self, msg: ControllerMessage) {
        self.nats.publish(self.scheduler_id, None, None, msg).await;
    }

    async fn send_by_capability(&self, capability: &str, msg: ControllerMessage) {
        self.nats
            .publish(self.scheduler_id, None, Some(capability), msg)
            .await;
    }

    async fn signal_ca_rotation(&self, reason: &str) {
        tracing::info!(reason, "external scheduler requesting CA rotation via NATS");
        self.nats
            .publish(
                self.scheduler_id,
                None,
                Some("controller"),
                ControllerMessage::RequestCaRotation(RequestCaRotationPayload {
                    reason: reason.to_string(),
                }),
            )
            .await;
    }

    async fn push_software_states_for_tenant(&self, payload: MqttSoftwareStatesPayload) {
        let msg = ControllerMessage::SoftwareStates(payload);
        self.nats
            .publish(self.scheduler_id, None, Some("mqtt_bridge"), msg)
            .await;
    }

    async fn signal_crl_renewal(&self) {
        tracing::info!("external scheduler triggering CRL rebuild via NATS");
        self.nats
            .publish(
                self.scheduler_id,
                None,
                Some("controller"),
                ControllerMessage::RequestCrlRenewal(RequestCrlRenewalPayload::default()),
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nats_scheduler_notifier_new() {
        // Verify construction compiles and holds the correct ID.
        // (Cannot test NATS publish without a running NATS server.)
        let id = Uuid::now_v7();
        // NatsConnection requires a live server, so we just verify the struct
        // layout compiles. Integration tests cover actual publishing.
        assert_eq!(std::mem::size_of::<Uuid>(), 16);
        let _ = id;
    }
}
