//! Agent-side infrastructure plugin extension point.
//!
//! Infrastructure plugins (e.g. Proxmox) can implement [`AgentInfraPlugin`] to
//! hook into the SSH agent's lifecycle without the agent knowing about the
//! specific infrastructure.  The trait covers:
//!
//! - **Bootstrap detection** — detect infrastructure after a host is bootstrapped.
//! - **Sync** — refresh infrastructure state during host sync.
//! - **Extension actions** — handle UI-driven actions.
//! - **Post-ReportHosts callbacks** — deferred operations after hosts are
//!   registered on the controller.
//! - **Plugin config response** — react to `ReportPluginConfigResponse`.
//! - **Migrations** — contribute agent-local DB tables.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use uptrakit_command::{CommandExecutor, RemoteExecutor};
use uptrakit_extension_framework::{
    ActionDef, ExtensionManifest, ExtensionRequestPayload, ExtensionResponsePayload,
};

use crate::SudoCommandEntry;
use crate::error::Result;

// ── Guest bootstrap callback ─────────────────────────────────────────────────

/// Parameters for bootstrapping a guest via an infrastructure plugin.
///
/// The plugin parses these from the extension action request. The SSH agent
/// performs the actual bootstrap using its own SSH transport and DB.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct GuestBootstrapParams {
    /// Local DB ID of the infrastructure host to use as gateway.
    pub gateway_host_id: String,
    /// Guest identifier (e.g. VMID for Proxmox).
    pub guest_id: u32,
    /// Guest type string (e.g. "lxc", "qemu").
    pub guest_type: String,
    /// Friendly name for the new host entry.
    pub name: String,
    /// Username to create/use on the guest.
    pub target_username: String,
    /// Write `NOPASSWD: ALL` instead of specific commands.
    pub allow_all: bool,
    /// Remove existing Uptrakit-managed keys before writing the new entry.
    pub remove_stale_keys: bool,
    /// Pre-generated UUID for the new host DB entry.
    pub host_id: uuid::Uuid,
    /// Service UUID for the `authorized_keys` comment.
    pub service_id: Option<uuid::Uuid>,
}

impl GuestBootstrapParams {
    /// Create a new [`GuestBootstrapParams`] with all required fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gateway_host_id: impl Into<String>,
        guest_id: u32,
        guest_type: impl Into<String>,
        name: impl Into<String>,
        target_username: impl Into<String>,
        allow_all: bool,
        remove_stale_keys: bool,
        host_id: uuid::Uuid,
        service_id: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            gateway_host_id: gateway_host_id.into(),
            guest_id,
            guest_type: guest_type.into(),
            name: name.into(),
            target_username: target_username.into(),
            allow_all,
            remove_stale_keys,
            host_id,
            service_id,
        }
    }
}

/// Result of a successful guest bootstrap.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct GuestBootstrapResult {
    /// The hostname/IP of the bootstrapped guest.
    pub guest_ip: String,
}

impl GuestBootstrapResult {
    /// Create a new [`GuestBootstrapResult`].
    pub fn new(guest_ip: impl Into<String>) -> Self {
        Self {
            guest_ip: guest_ip.into(),
        }
    }
}

/// Callback for performing the actual guest bootstrap.
///
/// The SSH agent implements this using its SSH transport, key generation, and
/// DB operations. Infrastructure plugins call it to bootstrap guests without
/// depending on agent-ssh internals.
#[async_trait]
pub trait GuestBootstrapExecutor: Send + Sync {
    /// Execute the guest bootstrap workflow.
    async fn bootstrap_guest(
        &self,
        params: GuestBootstrapParams,
    ) -> std::result::Result<GuestBootstrapResult, String>;
}

// ── Guest executor provider ──────────────────────────────────────────────────

/// Abstraction for creating executors that run commands inside infrastructure
/// guests (e.g. PVE LXC containers or QEMU VMs).
///
/// Infrastructure plugins implement this to provide guest execution without
/// the SSH agent depending on plugin-specific code. The agent uses these
/// executors for guest bootstrap and plugin compatibility probes.
#[async_trait]
pub trait GuestExecProvider: Send + Sync {
    /// Create a [`RemoteExecutor`] that runs commands inside the specified guest
    /// by routing through `gateway` (the infrastructure host's SSH connection).
    fn create_guest_remote_executor(
        &self,
        gateway: Arc<dyn RemoteExecutor>,
        guest_id: u32,
        guest_type: &str,
    ) -> Arc<dyn RemoteExecutor>;

    /// Create a [`CommandExecutor`] that runs [`CommandSpec`] commands inside
    /// the specified guest.
    fn create_guest_command_executor(
        &self,
        gateway: Arc<dyn RemoteExecutor>,
        guest_id: u32,
        guest_type: &str,
    ) -> Arc<dyn CommandExecutor>;

    /// Return the IP address of the specified guest.
    async fn get_guest_ip(
        &self,
        gateway: &dyn RemoteExecutor,
        guest_id: u32,
        guest_type: &str,
    ) -> std::result::Result<String, String>;
}

// ── Action invoker ───────────────────────────────────────────────────────────

/// Abstraction for invoking controller-side extension actions.
///
/// The SSH agent implements this by wrapping its `ServiceExtensionProxy`.
/// Infrastructure plugins receive a `&dyn InfraActionInvoker` and never depend
/// on `uptrakit-service-sdk` directly.
#[async_trait]
pub trait InfraActionInvoker: Send + Sync {
    /// Invoke an extension action on the controller.
    ///
    /// Returns the response payload on success, or a human-readable error
    /// string on failure (timeout, send failure, etc.).
    async fn invoke(
        &self,
        extension_id: &str,
        action_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<ExtensionResponsePayload, String>;
}

// ── Context ──────────────────────────────────────────────────────────────────

/// Context provided to infrastructure plugins by the SSH agent.
///
/// Bundles all state that plugin methods need without exposing agent internals.
pub struct InfraPluginContext<'a> {
    /// Agent-local SQLite database connection.
    pub db: &'a DatabaseConnection,
    /// Tenant ID (available after the agent has connected to the controller at
    /// least once).
    pub tenant_id: Option<&'a str>,
    /// Service UUID of this SSH agent instance.
    pub service_id: Option<uuid::Uuid>,
    /// Agent state directory (for DB init in spawned tasks).
    pub state_dir: &'a Path,
    /// DER-encoded ECIES private key for decrypting sensitive params.
    pub private_key_der: Option<&'a [u8]>,
    /// Invoker for calling controller-side extension actions.
    pub action_invoker: &'a dyn InfraActionInvoker,
    /// Executor for bootstrapping guests (SSH agent implements this).
    pub guest_bootstrap: &'a dyn GuestBootstrapExecutor,
}

// ── Return types ─────────────────────────────────────────────────────────────

/// Data needed to send a `ReportPluginConfig` wire message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfigReport {
    /// Plugin type string (e.g. `"infrastructure_proxmox"`).
    pub plugin_type: String,
    /// Human-readable config name (e.g. `"pve-<host_id>"`).
    pub name: String,
    /// Plugin-specific configuration JSON.
    pub config: serde_json::Value,
}

/// Result of infrastructure detection during host bootstrap.
#[derive(Debug, Default)]
pub struct BootstrapInfraResult {
    /// If `Some`, the agent should send `ReportPluginConfig` with this data.
    pub report_plugin_config: Option<PluginConfigReport>,
    /// If `Some`, the agent should update the host's `pve_plugin_config_id`
    /// directly (reusing an existing config from a cluster peer).
    pub existing_plugin_config_id: Option<String>,
    /// Whether any infrastructure was detected on this host.
    pub detected: bool,
}

/// A sudoers entry resolved by an infrastructure plugin during sync.
///
/// Mirrors `ResolvedSudoCommand` in agent-ssh but lives here so plugins can
/// return sudo requirements without depending on agent-ssh types.
#[derive(Debug, Clone)]
pub struct InfraResolvedSudo {
    /// Absolute command spec for the sudoers entry.
    ///
    /// The string is expressed in normal command-token form; the SSH agent
    /// escapes sudoers-special characters when rendering the file while
    /// preserving wildcard tokens such as `*`.
    pub command_path: String,
    /// Human-readable explanation for audit/documentation.
    pub explanation: String,
    /// Whether the entry needs `SETENV:` in the sudoers line.
    pub needs_setenv: bool,
}

/// Result of infrastructure sync for a host.
#[derive(Debug, Default)]
pub struct SyncInfraResult {
    /// Human-readable summary lines for CLI/extension output.
    pub summary_lines: Vec<String>,
    /// Additional sudoers entries the infrastructure requires on this host.
    pub sudo_commands: Vec<InfraResolvedSudo>,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Extension point for infrastructure plugins that hook into the SSH agent.
///
/// Each infrastructure plugin (e.g. Proxmox) implements this trait to integrate
/// with the SSH agent's bootstrap, sync, extension, and post-report lifecycle
/// without the agent knowing about the specific infrastructure.
#[async_trait]
pub trait AgentInfraPlugin: Send + Sync + 'static {
    /// Plugin type identifier (e.g. `"infrastructure_proxmox"`).
    fn plugin_type(&self) -> &str;

    /// Return SeaORM migrations for plugin-owned agent-local tables.
    ///
    /// The SSH agent runs these alongside its own migrations at startup.
    fn migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>>;

    /// Return additional extension manifests to register with the controller.
    ///
    /// These are merged into the SSH agent's extension registration payload.
    fn extension_manifests(&self) -> Vec<ExtensionManifest>;

    /// Return additional action definitions to register.
    fn extension_actions(&self) -> Vec<ActionDef>;

    /// Return action IDs that should appear in the SSH hosts data table's
    /// `primary_actions` list.
    fn primary_action_ids(&self) -> Vec<String>;

    /// Return required sudo command declarations for the plugin itself.
    ///
    /// These are used by the plugin registry's
    /// `all_required_sudo_commands()` aggregation. Infrastructure-specific
    /// sudo needs that depend on the remote host state (e.g. whether
    /// `/usr/sbin/pct` exists) should be returned from
    /// [`on_host_synced`](Self::on_host_synced) via
    /// [`SyncInfraResult::sudo_commands`] instead.
    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![]
    }

    /// Detect infrastructure during host bootstrap.
    ///
    /// Called after the standard bootstrap completes (user creation, key
    /// deployment, sudoers). The `executor` is connected to the bootstrapped
    /// host.
    async fn on_host_bootstrapped(
        &self,
        ctx: &InfraPluginContext<'_>,
        executor: &dyn uptrakit_command::RemoteExecutor,
        host_id: uuid::Uuid,
        host_name: &str,
    ) -> Result<BootstrapInfraResult>;

    /// Sync infrastructure state during host sync.
    ///
    /// Called during `sync-host` for hosts where this plugin previously
    /// detected infrastructure.
    async fn on_host_synced(
        &self,
        ctx: &InfraPluginContext<'_>,
        executor: &dyn uptrakit_command::RemoteExecutor,
        host_id: uuid::Uuid,
    ) -> Result<SyncInfraResult>;

    /// Check whether this plugin has infrastructure state for the given host.
    ///
    /// Used by the agent to decide whether to call [`on_host_synced`](Self::on_host_synced).
    async fn has_infra_state(&self, db: &DatabaseConnection, host_id: uuid::Uuid) -> bool;

    /// Handle a plugin-owned extension action.
    ///
    /// Return `Some(response)` if this plugin handles the action, or `None` to
    /// let the agent try the next plugin.
    async fn handle_extension_action(
        &self,
        ctx: &InfraPluginContext<'_>,
        request: &ExtensionRequestPayload,
    ) -> Option<ExtensionResponsePayload>;

    /// Called after `ReportHosts` has been sent to the controller.
    ///
    /// Plugins can drain deferred operations here (e.g. pending host-mapping
    /// matches that require the host to exist on the controller).
    async fn on_post_report_hosts(&self, ctx: &InfraPluginContext<'_>) -> Result<()>;

    /// Called when the controller responds to a `ReportPluginConfig` request
    /// that originated from this plugin's [`on_host_bootstrapped`](Self::on_host_bootstrapped).
    async fn on_plugin_config_reported(
        &self,
        db: &DatabaseConnection,
        plugin_config_id: uuid::Uuid,
        request_id: &str,
    ) -> Result<()>;

    /// Return a [`GuestExecProvider`] for executing commands inside infrastructure
    /// guests via this plugin, or `None` if the plugin does not support guest exec.
    fn guest_exec_provider(&self) -> Option<Arc<dyn GuestExecProvider>> {
        None
    }
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// Registry of agent-side infrastructure plugins.
///
/// The SSH agent creates this at startup via the plugin registry crate and
/// interacts with infrastructure plugins exclusively through this registry.
pub struct AgentInfraRegistry {
    plugins: Vec<Arc<dyn AgentInfraPlugin>>,
}

impl AgentInfraRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register an infrastructure plugin.
    pub fn register(&mut self, plugin: Arc<dyn AgentInfraPlugin>) {
        self.plugins.push(plugin);
    }

    /// Return a reference to all registered plugins.
    pub fn plugins(&self) -> &[Arc<dyn AgentInfraPlugin>] {
        &self.plugins
    }

    /// Collect all migrations from all registered plugins.
    pub fn all_migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        self.plugins.iter().flat_map(|p| p.migrations()).collect()
    }

    /// Collect all extension manifests from all registered plugins.
    pub fn all_extension_manifests(&self) -> Vec<ExtensionManifest> {
        self.plugins
            .iter()
            .flat_map(|p| p.extension_manifests())
            .collect()
    }

    /// Collect all action definitions from all registered plugins.
    pub fn all_extension_actions(&self) -> Vec<ActionDef> {
        self.plugins
            .iter()
            .flat_map(|p| p.extension_actions())
            .collect()
    }

    /// Collect all primary action IDs from all registered plugins.
    pub fn all_primary_action_ids(&self) -> Vec<String> {
        self.plugins
            .iter()
            .flat_map(|p| p.primary_action_ids())
            .collect()
    }

    /// Find the plugin that handles a given action ID in an extension request.
    pub fn find_action_handler(&self) -> &[Arc<dyn AgentInfraPlugin>] {
        &self.plugins
    }

    /// Return the [`GuestExecProvider`] from the plugin matching `plugin_type`, if any.
    pub fn guest_exec_provider_for(&self, plugin_type: &str) -> Option<Arc<dyn GuestExecProvider>> {
        self.plugins
            .iter()
            .find(|p| p.plugin_type() == plugin_type)
            .and_then(|p| p.guest_exec_provider())
    }
}

impl Default for AgentInfraRegistry {
    fn default() -> Self {
        Self::new()
    }
}
