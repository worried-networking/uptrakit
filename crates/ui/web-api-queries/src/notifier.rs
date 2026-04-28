//! Trait abstraction for sending messages to connected services.
//!
//! The concrete implementation (`NotificationService`) lives in `uptrakit-web-api`.
//! Query functions in this crate accept `&dyn ServiceNotifier` so that they do
//! not depend on Axum, NATS, or other web-api internals.

use uptrakit_wire::ControllerMessage;
use uuid::Uuid;

/// Abstraction over `NotificationService::send` for update dispatch.
///
/// Implementations deliver a `ControllerMessage` to the service identified by
/// `service_id`. Returns `true` if the target service was locally connected
/// at delivery time.
#[async_trait::async_trait]
pub trait ServiceNotifier: Send + Sync {
    /// Send a message to a specific service, returning whether it was locally connected.
    async fn send_to_service(&self, service_id: &Uuid, msg: ControllerMessage) -> bool;
}
