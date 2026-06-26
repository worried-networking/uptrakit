//! Unified capability-gated WebSocket handler for all service types.
//!
//! This module replaces the three separate handlers (`agent_ws`, `mqtt_ws`,
//! `ssh_agent_ws`) with a single pair of handler functions that dispatch
//! messages based on the service's persisted capability set.
//!
//! ## Background message processing
//!
//! Heavy message processing (DB queries, notifications, etc.) is offloaded
//! to a [`MessageProcessor`] task spawned per connection. The main loop
//! reads WebSocket frames, handles lightweight inline operations (Ping/Pong,
//! Disconnecting, Unknown, Close, rate limiting), and forwards everything
//! else to the processor via a bounded MPSC channel.
//!
//! The processor handles messages sequentially (preserving ordering) and
//! sends [`ProcessorResponse`](shared_types::ProcessorResponse) values back
//! to the main loop, which serializes and writes replies to the WebSocket
//! sink with `out_seq` staying in the main loop.
//!
//! # Public API
//!
//! - [`handle_authenticated_loop`] -- post-certificate operational loop.
//! - [`handle_enrolled_loop`] -- pre-certificate enrollment loop.
//! - [`trigger_discovery_for_agent_host`] -- send `DiscoverSoftware` to an
//!   agent for a specific host (also used by `hosts.rs`).

mod audit_service;
mod audit_surface;
mod cert;
mod credentials;
mod discovery;
mod embedded;
mod message_processor;
pub(super) mod messages;
mod reconnect;
mod renewal;
mod service_config;
mod session_authenticated;
mod session_enrolled;
mod shared_types;
mod surface_wire;
#[cfg(test)]
pub(super) mod test_support;
#[cfg(test)]
mod tests;
mod update_tracking;
mod updates;
mod workload;

pub(crate) use discovery::trigger_discovery_for_agent_host;
pub(crate) use embedded::{run_embedded_message_handler, run_embedded_system_message_handler};
pub(crate) use session_authenticated::handle_authenticated_loop;
pub(crate) use session_enrolled::handle_enrolled_loop;
use session_enrolled::upgrade_service_capabilities;
pub(crate) use updates::dispatch_next_batch_update;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum time to wait for a WebSocket write (`sink.send()`) to complete.
///
/// If a service stops reading from the WebSocket, the OS TCP send buffer fills
/// and `sink.send()` blocks indefinitely. This timeout bounds the hang so that
/// the handler loop can break and clean up the connection. Kept deliberately
/// shorter than the agent-side `SEND_TIMEOUT` (30 s) so the controller detects
/// the stuck connection first.
const WS_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Maximum consecutive unknown messages before closing the connection.
///
/// Prevents a misbehaving or fuzzing client from keeping a connection alive
/// indefinitely by sending only garbage message types. Resets on any known
/// message.
const MAX_CONSECUTIVE_UNKNOWN_MESSAGES: u32 = 10;
const MQTT_SERVICE_APP_NAME: &str = "uptrakit-mqtt";

fn system_service_tenant_binding(
    service_app_name: Option<&str>,
    default_tenant_id: uuid::Uuid,
) -> Option<uuid::Uuid> {
    (service_app_name == Some(MQTT_SERVICE_APP_NAME)).then_some(default_tenant_id)
}

pub(super) fn is_valid_service_config_scope(
    service_tenant_id: Option<uuid::Uuid>,
    payload_tenant_id: Option<uuid::Uuid>,
) -> bool {
    match service_tenant_id {
        Some(bound_tenant_id) => payload_tenant_id == Some(bound_tenant_id),
        None => true,
    }
}
