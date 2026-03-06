//! Proxmox VE infrastructure plugin for Uptrakit.
//!
//! This plugin communicates with the Proxmox VE REST API to discover VMs and
//! containers, then matches them to Uptrakit-managed hosts by hostname or IP
//! address. It operates on the controller side and exposes all functionality
//! through the Extensions framework.
//!
//! The `guest_exec` and `pve_setup` modules provide agent-side functionality
//! for executing commands inside PVE guests and bootstrapping PVE API credentials.

pub mod api_types;
pub mod client;
pub mod config;
pub mod discovery;
pub mod error;
pub mod extensions;
pub mod guest_exec;
pub mod matching;
pub mod plugin;
pub mod pve_setup;

pub use config::ProxmoxConfig;
pub use error::{ProxmoxError, Result};
pub use plugin::ProxmoxPlugin;
