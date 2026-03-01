use uptrakit_internal_wire::{ControllerMessage, MqttSoftwareStatesPayload};
use uuid::Uuid;

/// Abstraction over message delivery for scheduled task executors.
///
/// In-process (embedded scheduler) this wraps `NotificationService`.
/// Out-of-process (external scheduler) this publishes to NATS.
#[async_trait::async_trait]
pub trait SchedulerNotifier: Send + Sync {
    /// Send a message to a specific service.
    async fn send_to_service(&self, service_id: &Uuid, msg: ControllerMessage);

    /// Broadcast a message to all connected services.
    async fn broadcast(&self, msg: ControllerMessage);

    /// Send a message to all services with a specific capability.
    async fn send_by_capability(&self, capability: &str, msg: ControllerMessage);

    /// Signal the controller(s) to perform CA certificate rotation.
    async fn signal_ca_rotation(&self, reason: &str);

    /// Deliver pre-loaded software states to all MQTT services.
    ///
    /// The caller is responsible for loading the payload from the database
    /// before invoking this method, keeping the notification interface
    /// decoupled from the ORM layer.
    async fn push_software_states_for_tenant(&self, payload: MqttSoftwareStatesPayload);

    /// Signal all controller instances to rebuild the CRL immediately.
    ///
    /// The embedded-controller implementation fires `revocation_notify` locally
    /// and publishes `RequestCrlRenewal` to NATS for remote instances.
    /// The NATS-only implementation (external scheduler) publishes to NATS only.
    async fn signal_crl_renewal(&self);
}
