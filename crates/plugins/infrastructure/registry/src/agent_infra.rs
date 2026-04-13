//! Registry-owned surface for agent infrastructure plugin protocol types.
//!
//! This module provides a stable re-export path so downstream crates import
//! `uptrakit_plugin_infrastructure_registry::agent_infra::{...}` rather than
//! referencing `infrastructure-core` directly.

pub use uptrakit_plugin_infrastructure_core::agent_infra::{
    BootstrapInfraResult, GuestBootstrapExecutor, GuestBootstrapParams, GuestBootstrapResult,
    GuestExecProvider, InfraActionInvoker, InfraPluginContext, InfraResolvedSudo,
    PluginConfigReport, SyncInfraResult,
};
