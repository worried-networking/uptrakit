//! NATS publishing abstraction for cross-controller messaging.
//!
//! The [`NatsPublisher`] trait decouples [`crate::notification_service::NotificationService`]
//! and [`crate::event_broadcaster::EventBroadcaster`] (both in `uptrakit-web-api`) from the
//! concrete `NatsTransport` type so that controller-core can be tested without a NATS dependency.

use uptrakit_wire::ControllerMessage;
use uuid::Uuid;

/// Abstraction over NATS publishing used by `NotificationService` and `EventBroadcaster`.
///
/// Methods match exactly what `notification_service.rs` and `event_broadcaster.rs`
/// call on `NatsTransport`.  The concrete implementation lives in `uptrakit-web-api`
/// (`NatsTransport`) and delegates to the real NATS JetStream connection.
///
/// Fire-and-forget: implementors log errors internally and do not propagate them.
#[async_trait::async_trait]
pub trait NatsPublisher: Send + Sync {
    /// Publish a [`ControllerMessage`] to NATS JetStream.
    ///
    /// - `source_controller_id` — the publishing controller's UUID (used for
    ///   self-message filtering on the consumer side).
    /// - `target_service_id` — when `Some`, routes to the per-service subject;
    ///   when `None`, routes based on `target_capability`.
    /// - `target_capability` — when `Some`, routes to the capability subject
    ///   (e.g. `"update_tracking"` or `"controller"`); `None` broadcasts.
    /// - `msg` — the message to publish.
    async fn publish(
        &self,
        source_controller_id: Uuid,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: ControllerMessage,
    );
}
