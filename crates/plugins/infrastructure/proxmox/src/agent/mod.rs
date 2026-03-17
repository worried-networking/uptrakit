//! Agent-side Proxmox infrastructure plugin.
//!
//! This module provides the [`PluginBase`](uptrakit_plugin_infrastructure_core::PluginBase)
//! and subtrait implementations for Proxmox VE, encapsulating all PVE-specific
//! logic that runs inside the SSH agent.

pub mod db_ops;
pub mod entity;
pub mod extension_actions;
mod guest_exec_adapter;
pub mod migration;
pub mod plugin;

pub use guest_exec_adapter::ProxmoxGuestExecProvider;
