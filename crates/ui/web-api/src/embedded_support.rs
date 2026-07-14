//! Support for embedded services running inside the controller process.
//!
//! - [`EmbeddedServiceNotifier`] — callback interface for external service
//!   connect/disconnect events. Implemented by the controller's
//!   `EmbeddedServiceHost` and stored in `AppState`.
//! - [`run_embedded_message_handler`] — message processing loop for an
//!   embedded service's inbound messages, reusing the same dispatch pipeline
//!   as WebSocket-connected services.

use std::collections::BTreeSet;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use uptrakit_wire::{Capability, ServiceMessage};
use uuid::Uuid;

use crate::AppState;

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

/// Shared transport/session parameters for an embedded service's message
/// handler loop.
///
/// Bundles the fields common to [`run_embedded_message_handler`] and
/// [`run_embedded_system_message_handler`] so each function stays within the
/// `clippy::too_many_arguments` limit. The tenant-scoping argument (`Uuid` for
/// tenant services, `Option<Uuid>` for system services) is passed separately
/// since it is the one thing that differs between the two entry points.
#[non_exhaustive]
pub struct EmbeddedHandlerParams {
    pub state: Arc<AppState>,
    pub service_id: Uuid,
    pub connection_id: Uuid,
    pub capabilities: BTreeSet<Capability>,
    pub app_name: String,
    pub service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    pub cancel: CancellationToken,
}

impl EmbeddedHandlerParams {
    /// Construct a new set of embedded handler parameters.
    pub fn new(
        state: Arc<AppState>,
        service_id: Uuid,
        connection_id: Uuid,
        capabilities: BTreeSet<Capability>,
        app_name: String,
        service_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            state,
            service_id,
            connection_id,
            capabilities,
            app_name,
            service_rx,
            cancel,
        }
    }
}

/// Run the server-side message handler for an embedded service.
///
/// Reads [`ServiceMessage`] values from `service_rx` and dispatches them
/// through the same [`MessageProcessor`](crate::routes::service_ws::handler)
/// pipeline used by WebSocket-connected services. Replies are pushed back
/// through the [`ServiceConnectionRegistry`](crate::service_connections::ServiceConnectionRegistry),
/// reaching the embedded service via the response forwarder bridge.
///
/// This function blocks until `cancel` is triggered or the sender side of
/// `service_rx` is dropped.
pub async fn run_embedded_message_handler(params: EmbeddedHandlerParams, tenant_id: Uuid) {
    let EmbeddedHandlerParams {
        state,
        service_id,
        connection_id,
        capabilities,
        app_name,
        service_rx,
        cancel,
    } = params;
    crate::routes::service_ws::handler::run_embedded_message_handler(
        crate::routes::service_ws::handler::EmbeddedHandlerCallParams {
            state,
            service_id,
            connection_id,
            capabilities: &capabilities,
            app_name: &app_name,
            service_rx,
            cancel,
        },
        tenant_id,
    )
    .await;
}

/// Run the server-side message handler for an embedded system service.
pub async fn run_embedded_system_message_handler(
    params: EmbeddedHandlerParams,
    service_tenant_id: Option<Uuid>,
) {
    let EmbeddedHandlerParams {
        state,
        service_id,
        connection_id,
        capabilities,
        app_name,
        service_rx,
        cancel,
    } = params;
    crate::routes::service_ws::handler::run_embedded_system_message_handler(
        crate::routes::service_ws::handler::EmbeddedHandlerCallParams {
            state,
            service_id,
            connection_id,
            capabilities: &capabilities,
            app_name: &app_name,
            service_rx,
            cancel,
        },
        service_tenant_id,
    )
    .await;
}
