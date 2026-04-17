//! Proxmox VE infrastructure plugin for Uptrakit.
//!
//! [`ProxmoxPlugin`] is the unified plugin struct used on both the controller
//! and agent sides. On the controller it communicates with the Proxmox VE REST
//! API to discover VMs/CTs and match them to Uptrakit-managed hosts. On the
//! agent (with `agent-infra` feature) it implements `HostLifecycle`,
//! `HostReport`, and `GuestExec` subtraits.
//!
//! The `guest_exec` and `pve_setup` modules provide agent-side functionality
//! for executing commands inside PVE guests and bootstrapping PVE API credentials.
//!
//! The `agent` module (behind the `agent-infra` feature) provides the subtrait
//! implementations and supporting DB/surface logic that hook into the SSH
//! agent's lifecycle.

#[cfg(feature = "agent-infra")]
pub mod agent;
pub mod api_types;
pub mod client;
pub mod config;
#[cfg(feature = "migrations")]
pub mod controller_migration;
pub mod discovery;
pub mod error;
pub mod guest_exec;
pub mod matching;
pub mod plugin;
#[cfg(not(feature = "agent-infra"))]
pub mod policy_store;
pub mod pve_setup;
pub mod surfaces;
#[cfg(not(feature = "agent-infra"))]
pub mod update_protection;

pub use config::ProxmoxConfig;
pub use error::{ProxmoxError, Result};
pub use plugin::{DESCRIPTOR, ProxmoxPlugin};
