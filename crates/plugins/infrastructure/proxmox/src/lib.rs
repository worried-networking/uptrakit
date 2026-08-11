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
pub(crate) mod db_migrate;
pub mod discovery;
pub(crate) mod entity;
pub mod error;
pub mod guest_exec;
pub mod matching;
#[cfg(all(test, feature = "migrations"))]
mod matching_isolation_tests;
pub mod plugin;
pub mod policy_store;
#[cfg(all(test, feature = "migrations"))]
mod policy_store_sqlite_tests;
pub(crate) mod protection_store;
pub mod pve_setup;
pub(crate) mod reset;
#[cfg(feature = "plugin-ops")]
pub(crate) mod resource_scaling;
pub(crate) mod scaling_mode;
pub mod scaling_store;
pub mod surfaces;
/// Test-only helpers for in-tree functional tests.
///
/// Gated on `feature = "testing"`. Exposes typed insertion helpers for
/// proxmox-specific tables whose entities live in the crate-internal
/// `entity` module. Not part of the stable public API; signatures and
/// contracts may change without semver impact, and out-of-tree callers
/// are unsupported.
#[cfg(feature = "testing")]
pub mod testing;
pub mod update_protection;

pub use config::ProxmoxConfig;
pub use error::{ProxmoxError, Result};
pub use plugin::{DESCRIPTOR, ProxmoxPlugin};
