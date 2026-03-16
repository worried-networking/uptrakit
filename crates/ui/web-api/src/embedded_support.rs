//! Trait for notifying the embedded service infrastructure about external
//! service connections.
//!
//! Implemented by the controller's `EmbeddedServiceHost`. Uses only wire
//! types (from `uptrakit-internal-wire`) so that `web-api` does not depend
//! on any service-specific crate.

use std::collections::BTreeSet;

use uptrakit_internal_wire::Capability;
use uuid::Uuid;

/// Callback interface for external service connect/disconnect events.
///
/// The controller's `EmbeddedServiceHost` implements this trait and is stored
/// in `AppState` as `Option<Arc<dyn EmbeddedServiceNotifier>>`. The WS handler
/// calls these methods at the appropriate lifecycle points so that embedded
/// services can yield or resume.
pub trait EmbeddedServiceNotifier: Send + Sync {
    /// An external service has connected via WebSocket.
    fn on_external_connected(
        &self,
        service_id: Uuid,
        capabilities: &BTreeSet<Capability>,
        hostname: Option<&str>,
        is_system: bool,
    );

    /// An external service has disconnected.
    fn on_external_disconnected(&self, service_id: &Uuid);

    /// A `ReportHosts` message provided a machine_id for an external service.
    fn on_machine_id_reported(&self, service_id: &Uuid, machine_id: &str);

    /// Check whether a specific capability is currently yielded by an
    /// embedded service (because an external service provides it).
    fn is_capability_yielded(&self, capability: &Capability) -> bool;
}
