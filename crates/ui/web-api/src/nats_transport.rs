//! NATS JetStream transport for cross-controller messaging.
//!
//! This module provides NATS-based pub/sub for delivering
//! [`ControllerMessage`](uptrakit_internal_wire::ControllerMessage)s across
//! multiple controller instances. Only compiled when the `nats` feature is
//! enabled.

use uptrakit_internal_wire::ControllerMessage;
use uuid::Uuid;

/// NATS transport handle used by [`NotificationService`](crate::notification_service::NotificationService)
/// to publish messages across controllers.
#[derive(Clone)]
pub struct NatsTransport {
    _private: (), // placeholder; fields added in Step 3
}

impl NatsTransport {
    /// Publish a message to NATS JetStream.
    ///
    /// Fire-and-forget: errors are logged, not propagated.
    pub async fn publish(
        &self,
        _source_controller_id: Uuid,
        _target_service_id: Option<Uuid>,
        _target_capability: Option<&str>,
        _msg: ControllerMessage,
    ) {
        // Stub — will be implemented in the NATS transport step.
    }
}
