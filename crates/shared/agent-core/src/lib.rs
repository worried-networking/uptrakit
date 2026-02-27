//! Shared agent logic for Uptrakit agents.
//!
//! This crate provides the shared version-check and update-execution
//! primitives used by both `uptrakit-agent` (local command execution) and
//! `uptrakit-agent-ssh` (remote SSH command execution). The executor is
//! injected by the caller, keeping transport details outside this crate.

pub mod client;
pub mod connection_context;
pub mod error;
pub mod update;
pub mod version_check;

// ── Public re-exports ────────────────────────────────────────────────────────

pub use client::{
    InFlightUpdate, UpdateEvent, handle_check_versions, handle_discover_software,
    handle_execute_update, handle_graceful_shutdown, send_update_output, send_update_result,
    start_update,
};
pub use connection_context::ConnectionContext;
pub use update::{UpdateExecutionResult, UpdateOutputMessage};
pub use version_check::{VersionCheckOutcome, check_version};
