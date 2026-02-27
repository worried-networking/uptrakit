use uptrakit_internal_wire::ControllerMessage;
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

    /// Load software states for a tenant and push to all MQTT services.
    async fn push_software_states_for_tenant(
        &self,
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
    );
}
