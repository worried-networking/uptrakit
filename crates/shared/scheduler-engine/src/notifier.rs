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

    /// Signal that software states have changed for a tenant.
    ///
    /// The embedded-controller implementation loads states and pushes them
    /// directly via `NotificationService`. The NATS-only implementation
    /// (external scheduler) publishes a `SoftwareStatesChanged` signal to
    /// the controller NATS subject; the receiving controller performs the
    /// actual load-and-push.
    async fn signal_software_states_changed(&self, tenant_id: Uuid);

    /// Signal all controller instances to rebuild the CRL immediately.
    ///
    /// The embedded-controller implementation fires `revocation_notify` locally
    /// and publishes `RequestCrlRenewal` to NATS for remote instances.
    /// The NATS-only implementation (external scheduler) publishes to NATS only.
    async fn signal_crl_renewal(&self);
}

/// No-op implementation of [`SchedulerNotifier`] for use in unit tests.
///
/// All methods are no-ops. Use this when the test only needs the executor to
/// run without actually delivering any messages.
#[cfg(test)]
pub(crate) struct NoopSchedulerNotifier;

#[cfg(test)]
#[async_trait::async_trait]
impl SchedulerNotifier for NoopSchedulerNotifier {
    async fn send_to_service(&self, _service_id: &Uuid, _msg: ControllerMessage) {}
    async fn broadcast(&self, _msg: ControllerMessage) {}
    async fn send_by_capability(&self, _capability: &str, _msg: ControllerMessage) {}
    async fn signal_ca_rotation(&self, _reason: &str) {}
    async fn signal_software_states_changed(&self, _tenant_id: Uuid) {}
    async fn signal_crl_renewal(&self) {}
}
