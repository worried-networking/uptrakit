//! Proxmox VE infrastructure plugin for Uptrakit.
//!
//! This plugin communicates with the Proxmox VE REST API to discover VMs and
//! containers, then matches them to Uptrakit-managed hosts by hostname or IP
//! address. It operates entirely on the controller side (no agent-side
//! capabilities) and exposes all functionality through the Extensions framework.

pub mod api_types;
pub mod client;
pub mod config;
pub mod discovery;
pub mod error;
pub mod extensions;
pub mod matching;
pub mod plugin;

pub use config::ProxmoxConfig;
pub use error::{ProxmoxError, Result};
pub use plugin::ProxmoxPlugin;
