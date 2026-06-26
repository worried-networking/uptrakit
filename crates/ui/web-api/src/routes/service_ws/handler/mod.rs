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
pub(crate) use updates::dispatch_next_batch_update;
