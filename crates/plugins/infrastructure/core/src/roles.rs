//! Role traits for the unified plugin framework.
//!
//! Each role trait represents a specific capability. Plugins implement the
//! traits matching their declared roles. The `declare_plugin!` macro generates
//! compile-time assertions that the plugin struct implements all declared roles.
//!
//! # Trait rename mapping
//!
//! | Old name | New name |
//! |----------|----------|
//! | `PluginBase` | `PluginMeta` + role traits + `PluginDescriptor` |
//! | `DiscoveryPlugin` | [`Discoverer`] |
//! | `VersionDetectorPlugin` | [`VersionDetector`] |
//! | `ReleaseFetcherPlugin` | [`ReleaseFetcher`] |
//! | `PackageIndexPlugin` | [`PackageIndexer`] |
//! | `UpdateExecutorPlugin` | [`UpdateExecutor`] |
//! | `UpdateLifecyclePlugin` | [`LifecycleHook`] |
//! | `NotificationTransportPlugin` | [`NotificationTransport`] |
//! | `SoftwareItemLifecyclePlugin` | [`SoftwareItemLifecycle`] |
//! | `HostLifecyclePlugin` | [`HostLifecycle`] |
//! | `HostReportPlugin` | [`HostReport`] |
//! | `GuestExecPlugin` | [`GuestExec`] |

use std::collections::HashMap;
#[cfg(feature = "agent-infra")]
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uptrakit_shared_types::PluginTypeId;
use uuid::Uuid;

use crate::UpdateOutputSender;
use crate::batch_detect::{BatchDetectItem, BatchDetectResult};
use crate::batch_fetch::{BatchFetchItem, BatchFetchResult};
use crate::batch_update::{BatchUpdateItem, BatchUpdateResult};
use crate::error::Result;
use crate::traits::{HostCompatibility, PreUpdateHookResult, UpdateLifecycleContext};
use crate::types::{DiscoveredSoftware, ReleaseInfo, UpstreamRelease};
use crate::version::Version;
#[cfg(feature = "agent-infra")]
use uptrakit_surfaces::{SurfaceActionRequest, SurfaceActionResponse};

// ── PluginMeta ──────────────────────────────────────────────────────────────

/// Common identity trait for all plugins.
///
/// Every plugin struct implements this. Returns the plugin's type ID as a typed
/// `PluginTypeId` (zero allocation for well-known constants via `from_static`).
pub trait PluginMeta: Send + Sync + 'static {
    /// Returns the plugin's type ID.
    fn plugin_type_id(&self) -> PluginTypeId;
}

// ── Per-instance software/hook roles ────────────────────────────────────────

/// Discover software that a plugin can manage on the local system.
#[async_trait]
pub trait Discoverer: PluginMeta {
    /// Discover manageable software on the local system.
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>>;

    /// Detect whether this plugin is applicable to the current host.
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        Ok(HostCompatibility::Compatible)
    }
}

/// Detect installed versions of software packages.
#[async_trait]
pub trait VersionDetector: PluginMeta {
    /// Detect the currently installed version of a package.
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>>;

    /// Detect installed versions for multiple packages in one operation.
    ///
    /// Default falls back to sequential calls. Override for batch efficiency.
    async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            match self
                .detect_installed_version(&item.package_identifier)
                .await
            {
                Ok(v) => results.push(BatchDetectResult {
                    package_identifier: item.package_identifier.clone(),
                    installed_version: v,
                    error: None,
                    display_version: None,
                }),
                Err(e) => results.push(BatchDetectResult {
                    package_identifier: item.package_identifier.clone(),
                    installed_version: None,
                    error: Some(e.to_string()),
                    display_version: None,
                }),
            }
        }
        Ok(results)
    }
}

/// Fetch upstream releases for software packages.
#[async_trait]
pub trait ReleaseFetcher: PluginMeta {
    /// Fetch available releases from the upstream source.
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>>;

    /// Fetch releases for multiple packages in one operation.
    ///
    /// Default falls back to sequential calls. Override for batch efficiency.
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            match self.fetch_releases(&item.package_identifier).await {
                Ok(releases) => results.push(BatchFetchResult {
                    package_identifier: item.package_identifier.clone(),
                    releases,
                    error: None,
                }),
                Err(e) => results.push(BatchFetchResult {
                    package_identifier: item.package_identifier.clone(),
                    releases: vec![],
                    error: Some(e.to_string()),
                }),
            }
        }
        Ok(results)
    }
}

/// Refresh/sync the local package index from remote sources.
#[async_trait]
pub trait PackageIndexer: PluginMeta {
    /// Refresh the local package index (e.g. `apt update`, `brew update`).
    async fn refresh_package_index(&self) -> Result<()>;
}

/// Result returned by [`UpdateExecutor::execute_update`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ExecuteUpdateResult {
    pub output: String,
    /// When `true`, the controller will transition this update to `AwaitingRestart`
    /// instead of `Completed`. The plugin decides this based on what actually happened
    /// (e.g., the shell plugin's `resumable: true` config, or APT detecting a reboot is needed).
    pub resumable: bool,
}

impl ExecuteUpdateResult {
    /// Construct an update result.
    pub fn new(output: impl Into<String>, resumable: bool) -> Self {
        Self {
            output: output.into(),
            resumable,
        }
    }
}

/// Execute software updates.
#[async_trait]
pub trait UpdateExecutor: PluginMeta {
    /// Execute a single package update with streaming output.
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        release_info: Option<&ReleaseInfo>,
        output_tx: &UpdateOutputSender,
    ) -> Result<ExecuteUpdateResult>;

    /// Execute updates for multiple packages in a single operation.
    ///
    /// Default falls back to sequential calls. Override for batch efficiency.
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &UpdateOutputSender,
    ) -> Result<Vec<BatchUpdateResult>> {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            match self
                .execute_update(
                    &item.package_identifier,
                    &item.to_version,
                    item.release_info.as_ref(),
                    output_tx,
                )
                .await
            {
                Ok(result) => results.push(BatchUpdateResult {
                    package_identifier: item.package_identifier.clone(),
                    success: true,
                    // resumable is intentionally not propagated; batch callers handle
                    // resumability at the batch level via BatchUpdateResult.
                    output: result.output,
                }),
                Err(e) => results.push(BatchUpdateResult {
                    package_identifier: item.package_identifier.clone(),
                    success: false,
                    output: format!("{e}"),
                }),
            }
        }
        Ok(results)
    }
}

/// Standalone update lifecycle hooks (pre/post update).
#[async_trait]
pub trait LifecycleHook: PluginMeta {
    /// Run before an update is applied. May abort the update.
    async fn execute_pre_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
    ) -> Result<PreUpdateHookResult>;

    /// Run after an update has been applied. Errors are logged, not fatal.
    async fn execute_post_hook(
        &self,
        ctx: &UpdateLifecycleContext,
        output_tx: &UpdateOutputSender,
    ) -> Result<()>;
}

// ── Singleton roles ─────────────────────────────────────────────────────────

/// Notification delivery channel (webhook, Telegram, email, etc.).
///
/// Singleton created at catalog construction. The descriptor's `type_id` is the
/// single authoritative key — no `channel_type()` method.
#[async_trait]
pub trait NotificationTransport: PluginMeta {
    /// Deliver a pre-built message using the given channel-specific config.
    async fn deliver(
        &self,
        config: &serde_json::Value,
        settings: &serde_json::Value,
        message: &uptrakit_notification_plugin_core::DeliveryMessage,
    ) -> uptrakit_notification_plugin_core::Result<()>;
}

/// Typed paging request for listing notification channels for a plugin type.
#[derive(Debug, Clone, Copy)]
pub struct NotificationChannelListRequest<'a> {
    pub tenant_id: Uuid,
    pub channel_type: &'a str,
    pub page: u64,
    pub per_page: u64,
}

/// Typed notification-channel row used by first-wave controller surface actions.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationChannelListItem {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub created_at_rfc3339: String,
    pub config: serde_json::Value,
}

/// Typed paginated channel list response for controller surface actions.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationChannelListPage {
    pub items: Vec<NotificationChannelListItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
    pub total_pages: u64,
}

/// Notification action-token state used by Telegram callback handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationActionTokenRecord {
    pub action_token: Uuid,
    pub action_taken: Option<String>,
}

/// Typed notification-channel persistence boundary for controller-side actions.
#[async_trait]
pub trait NotificationChannelStore: Send + Sync {
    /// List channels by tenant + channel type using typed paging inputs.
    async fn list_channels(
        &self,
        req: NotificationChannelListRequest<'_>,
    ) -> Result<NotificationChannelListPage>;

    /// Resolve a notification action token to its current delivery state.
    async fn resolve_action_token(
        &self,
        action_token: Uuid,
    ) -> Result<Option<NotificationActionTokenRecord>>;

    /// Mark an action token as triggered if it exists and is not already set.
    async fn mark_action_token_triggered(&self, action_token: Uuid) -> Result<()>;
}

/// Typed global Telegram settings boundary for controller-side surface actions.
#[async_trait]
pub trait TelegramGlobalSettingsStore: Send + Sync {
    /// Load the global Telegram bot token.
    async fn load_global_bot_token(&self) -> Result<String>;

    /// Save the global Telegram bot token and return the stored value.
    async fn save_global_bot_token(&self, bot_token: String) -> Result<String>;
}

/// Typed list request for paginated Proxmox host mappings.
///
/// `plugin_config_id` is optional: when absent, mappings for all Proxmox
/// configurations belonging to the tenant are returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxHostMappingsRequest {
    pub plugin_config_id: Option<Uuid>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// Typed config-scoped Proxmox action request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxPluginConfigRequest {
    pub plugin_config_id: Uuid,
}

/// Typed manual-match request for Proxmox host mappings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxManualMatchRequest {
    pub mapping_id: Uuid,
    pub host_id: Uuid,
}

/// Typed approve-match request for Proxmox host mappings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxApproveMatchRequest {
    pub mapping_id: Uuid,
    pub host_id: Uuid,
    pub match_method: String,
}

/// Typed mapping-targeted Proxmox action request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxMappingRequest {
    pub mapping_id: Uuid,
}

/// Typed host-targeted Proxmox action request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxHostInfoRequest {
    pub host_id: Uuid,
}

/// Typed list request for unmatched Proxmox guests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxUnmatchedGuestsRequest {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

/// Typed scope selector for Proxmox policy UI actions.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProxmoxScopeSelectionRequest {
    pub plugin_config_id: Option<Uuid>,
    pub software_item_id: Option<Uuid>,
}

/// Typed preload request for Proxmox software-item overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxItemOverridePreloadRequest {
    pub software_item_id: Uuid,
    pub plugin_config_id: Option<Uuid>,
}

/// Typed save request for Proxmox global-default policies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxGlobalDefaultsSaveRequest {
    pub plugin_config_id: Uuid,
    pub mode: String,
    pub backup_target_option: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
}

/// Typed save request for Proxmox software-item override policies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxmoxItemOverrideSaveRequest {
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub mode: String,
    pub backup_target_option: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
}

/// Typed Proxmox surface-action boundary for host-mapping and policy UI actions.
#[async_trait]
pub trait ProxmoxSurfaceStore: Send + Sync {
    async fn list_host_mappings(
        &self,
        request: ProxmoxHostMappingsRequest,
    ) -> Result<serde_json::Value>;
    async fn discover_hosts(
        &self,
        request: ProxmoxPluginConfigRequest,
    ) -> Result<serde_json::Value>;
    async fn test_connection(
        &self,
        request: ProxmoxPluginConfigRequest,
    ) -> Result<serde_json::Value>;
    async fn match_host(&self, request: ProxmoxManualMatchRequest) -> Result<serde_json::Value>;
    async fn approve_match(&self, request: ProxmoxApproveMatchRequest)
    -> Result<serde_json::Value>;
    async fn unmatch_host(&self, request: ProxmoxMappingRequest) -> Result<serde_json::Value>;
    async fn list_all_unmatched(
        &self,
        request: ProxmoxUnmatchedGuestsRequest,
    ) -> Result<serde_json::Value>;
    async fn get_host_info(&self, request: ProxmoxHostInfoRequest) -> Result<serde_json::Value>;

    async fn preload_global_defaults(
        &self,
        request: ProxmoxScopeSelectionRequest,
    ) -> Result<serde_json::Value>;
    async fn save_global_defaults(
        &self,
        request: ProxmoxGlobalDefaultsSaveRequest,
    ) -> Result<serde_json::Value>;
    async fn preload_item_overrides(
        &self,
        request: ProxmoxItemOverridePreloadRequest,
    ) -> Result<serde_json::Value>;
    async fn save_item_overrides(
        &self,
        request: ProxmoxItemOverrideSaveRequest,
    ) -> Result<serde_json::Value>;
    async fn load_backup_target_options(
        &self,
        request: ProxmoxScopeSelectionRequest,
    ) -> Result<serde_json::Value>;
}

/// Typed Proxmox host mapping required by update-protection workflows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxmoxHostMappingRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Option<Uuid>,
    pub plugin_config_id: Uuid,
    pub proxmox_node: String,
    pub proxmox_vmid: i64,
    pub proxmox_type: String,
}

/// Typed protection mode for Proxmox controller protection workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxmoxProtectionMode {
    #[default]
    DoNothing,
    Snapshot,
    Backup,
}

/// Typed effective protection policy used during pre-update planning.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxmoxProtectionPolicyRecord {
    pub mode: ProxmoxProtectionMode,
    pub backup_target_key: Option<String>,
    pub snapshot_timeout_seconds: Option<i64>,
    pub backup_timeout_seconds: Option<i64>,
}

/// Typed persisted audit row used by Proxmox protection reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxmoxProtectionAuditRecord {
    pub update_history_id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub plugin_config_id: Uuid,
    pub mapping_id: Option<Uuid>,
    pub mode: ProxmoxProtectionMode,
    pub status: String,
    pub artifact_kind: Option<String>,
    pub artifact_ref: Option<String>,
    pub backup_target_key: Option<String>,
    pub detail: Option<String>,
    pub error_message: Option<String>,
}

/// Typed Proxmox protection persistence boundary for controller-side workflows.
#[async_trait]
pub trait ProxmoxProtectionStore: Send + Sync {
    /// Load the Proxmox host mapping for a tenant/host pair.
    async fn load_host_mapping(
        &self,
        tenant_id: Uuid,
        host_id: Uuid,
    ) -> Result<Option<ProxmoxHostMappingRecord>>;

    /// Load raw Proxmox plugin config JSON for a tenant-scoped plugin config row.
    async fn load_plugin_config_payload(
        &self,
        tenant_id: Uuid,
        plugin_config_id: Uuid,
    ) -> Result<serde_json::Value>;

    /// Load the effective update-protection policy for a software item.
    async fn load_effective_policy(
        &self,
        tenant_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
    ) -> Result<ProxmoxProtectionPolicyRecord>;

    /// Load persisted protection audit state for a dispatch row.
    async fn load_audit(
        &self,
        update_history_id: Uuid,
    ) -> Result<Option<ProxmoxProtectionAuditRecord>>;

    /// Upsert protection audit state for a dispatch row.
    async fn upsert_audit(&self, audit: &ProxmoxProtectionAuditRecord) -> Result<()>;

    /// Resolve a cached backup target by plugin config and logical key.
    async fn find_cached_backup_target(
        &self,
        plugin_config_id: Uuid,
        target_key: &str,
    ) -> Result<Option<String>>;
}

/// Typed controller boundary for surface-action handlers.
pub trait SurfaceActionController: Send + Sync {
    /// Authenticated tenant scope for this action.
    fn tenant_id(&self) -> Uuid;

    /// Authenticated user ID when available.
    fn user_id(&self) -> Option<Uuid>;

    /// Tenant-scoped database access — the sole persistence seam for plugin surface actions.
    ///
    /// Only available when the `plugin-ops` feature is active.
    #[cfg(feature = "plugin-ops")]
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;

    /// Notification channel persistence capability.
    fn notification_channel_store(&self) -> Option<&dyn NotificationChannelStore> {
        None
    }

    /// Global Telegram settings capability.
    fn telegram_global_settings_store(&self) -> Option<&dyn TelegramGlobalSettingsStore> {
        None
    }

    /// Proxmox host/policy surface-actions capability.
    fn proxmox_surface_store(&self) -> Option<&dyn ProxmoxSurfaceStore> {
        None
    }

    /// Proxmox update-protection persistence capability.
    fn proxmox_protection_store(&self) -> Option<&dyn ProxmoxProtectionStore> {
        None
    }
}

/// Typed controller boundary for pre/post update protection workflows.
pub trait UpdateProtectionController: Send + Sync {
    /// Tenant-scoped database access for the update protection workflow.
    #[cfg(feature = "plugin-ops")]
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;

    /// Proxmox update-protection persistence capability.
    fn proxmox_protection_store(&self) -> Option<&dyn ProxmoxProtectionStore> {
        None
    }
}

/// Controller-side pre-update protection workflow plugin.
///
/// Singleton created at catalog construction.
#[async_trait]
pub trait ControllerUpdateProtection: PluginMeta {
    /// Prepare protection artifacts before update execution.
    async fn prepare_pre_update_protection(
        &self,
        ctx: &ControllerProtectionContext<'_>,
    ) -> Result<ControllerProtectionDecision>;

    /// Finalize post-update state (success/failure reconciliation).
    async fn finalize_post_update(
        &self,
        ctx: &ControllerPostUpdateContext<'_>,
    ) -> Result<PostUpdateOutcome>;
}

/// Context provided before update execution.
#[non_exhaustive]
pub struct ControllerProtectionContext<'a> {
    /// Typed controller boundary for data access/capabilities.
    pub controller: &'a dyn UpdateProtectionController,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    /// Optional channel for streaming protection status lines to the orchestrator.
    /// `None` for batch and recovery callers; `Some` when called from the orchestrator.
    pub output_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
}

impl<'a> ControllerProtectionContext<'a> {
    /// Construct a controller pre-update protection context.
    pub fn new(
        controller: &'a dyn UpdateProtectionController,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        update_history_id: Uuid,
    ) -> Self {
        Self {
            controller,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
            output_tx: None,
        }
    }

    /// Attach an output sender so the plugin can stream status lines to the orchestrator.
    pub fn with_output_tx(mut self, tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>) -> Self {
        self.output_tx = Some(tx);
        self
    }
}

/// Outcome of pre-update protection preparation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ControllerProtectionDecision {
    pub attempted: bool,
    pub succeeded: bool,
    pub protection_status: Option<String>,
    pub protection_summary: Option<String>,
}

impl ControllerProtectionDecision {
    /// Build a decision for a no-op (`attempted = false`) protection path.
    pub fn skipped(summary: Option<String>) -> Self {
        Self {
            attempted: false,
            succeeded: true,
            protection_status: None,
            protection_summary: summary,
        }
    }

    /// Build a successful protection decision.
    pub fn success(status: Option<String>, summary: Option<String>) -> Self {
        Self {
            attempted: true,
            succeeded: true,
            protection_status: status,
            protection_summary: summary,
        }
    }

    /// Build a failed protection decision.
    pub fn failure(status: Option<String>, summary: Option<String>) -> Self {
        Self {
            attempted: true,
            succeeded: false,
            protection_status: status,
            protection_summary: summary,
        }
    }
}

/// Context provided after update execution.
#[non_exhaustive]
pub struct ControllerPostUpdateContext<'a> {
    /// Typed controller boundary for data access/capabilities.
    pub controller: &'a dyn UpdateProtectionController,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    pub final_status: uptrakit_shared_types::UpdateStatus,
}

impl<'a> ControllerPostUpdateContext<'a> {
    /// Construct a controller post-update finalization context.
    pub fn new(
        controller: &'a dyn UpdateProtectionController,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        update_history_id: Uuid,
        final_status: uptrakit_shared_types::UpdateStatus,
    ) -> Self {
        Self {
            controller,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
            final_status,
        }
    }
}

/// Post-update reconciliation result.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct PostUpdateOutcome {
    pub recovery_hint: Option<String>,
}

impl PostUpdateOutcome {
    /// Construct a post-update outcome.
    pub fn new(recovery_hint: Option<String>) -> Self {
        Self { recovery_hint }
    }
}

// ── Software item lifecycle types ────────────────────────────────────────

/// Snapshot of a just-created software item, decoupled from SeaORM.
///
/// Plugins receive this as an immutable reference so they can inspect the
/// item without accessing the database.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SoftwareItemCreatedEvent {
    pub id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub name: String,
    pub featured: bool,
    pub icon_url: Option<String>,
}

impl SoftwareItemCreatedEvent {
    /// Create a new event snapshot.
    pub fn new(
        id: uuid::Uuid,
        tenant_id: uuid::Uuid,
        name: String,
        featured: bool,
        icon_url: Option<String>,
    ) -> Self {
        Self {
            id,
            tenant_id,
            name,
            featured,
            icon_url,
        }
    }
}

/// Patch returned by a software item lifecycle plugin.
///
/// Only `Some` fields are applied to the database row. This uses the
/// `Option<Option<T>>` pattern: `Some(Some(url))` = set, `Some(None)` = clear,
/// `None` = no change.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SoftwareItemPatch {
    pub icon_url: Option<Option<String>>,
}

/// Pre-resolved context for software-item lifecycle hooks.
///
/// The caller populates plugin type settings once and forwards them to all
/// lifecycle plugins without per-plugin I/O during dispatch.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SoftwareItemLifecycleContext {
    type_settings: HashMap<PluginTypeId, serde_json::Value>,
}

impl SoftwareItemLifecycleContext {
    /// Create a context from preloaded type settings.
    pub fn new(type_settings: HashMap<PluginTypeId, serde_json::Value>) -> Self {
        Self { type_settings }
    }

    /// Insert or replace type settings for a plugin type.
    pub fn insert_type_setting(&mut self, plugin_type: PluginTypeId, config: serde_json::Value) {
        self.type_settings.insert(plugin_type, config);
    }

    /// Returns the raw type settings JSON for the given plugin type.
    pub fn type_setting(&self, plugin_type: &PluginTypeId) -> Option<&serde_json::Value> {
        self.type_settings.get(plugin_type)
    }

    /// Deserialize type settings for a plugin type into a strongly typed model.
    ///
    /// Returns `None` when settings are absent or JSON fails to deserialize.
    pub fn typed_type_setting<T: DeserializeOwned>(&self, plugin_type: &PluginTypeId) -> Option<T> {
        let raw = self.type_setting(plugin_type)?;
        match serde_json::from_value(raw.clone()) {
            Ok(parsed) => Some(parsed),
            Err(err) => {
                tracing::warn!(
                    plugin_type = %plugin_type,
                    error = %err,
                    "failed to deserialize software item lifecycle type settings"
                );
                None
            }
        }
    }
}

impl SoftwareItemPatch {
    /// Create an empty patch (no changes).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the `icon_url` field.
    pub fn with_icon_url(mut self, icon_url: Option<String>) -> Self {
        self.icon_url = Some(icon_url);
        self
    }

    /// Returns `true` when no fields are set.
    pub fn is_empty(&self) -> bool {
        self.icon_url.is_none()
    }
}

/// Plugins that react to software item lifecycle events.
///
/// Singleton created at catalog construction. Controller-only.
#[async_trait]
pub trait SoftwareItemLifecycle: PluginMeta {
    /// Called after a software item is created.
    async fn on_software_item_created(
        &self,
        event: &SoftwareItemCreatedEvent,
        ctx: &SoftwareItemLifecycleContext,
    ) -> std::result::Result<Option<SoftwareItemPatch>, crate::error::PluginError>;
}

// ── Infrastructure roles (agent-side) ───────────────────────────────────────

/// Infrastructure host lifecycle hooks (bootstrap, sync).
#[cfg(feature = "agent-infra")]
#[async_trait]
pub trait HostLifecycle: PluginMeta {
    /// Detect infrastructure during host bootstrap.
    async fn on_host_bootstrapped(
        &self,
        ctx: &crate::agent_infra::InfraPluginContext<'_>,
        executor: &dyn uptrakit_command::RemoteExecutor,
        host_id: uuid::Uuid,
        host_name: &str,
    ) -> Result<crate::agent_infra::BootstrapInfraResult>;

    /// Sync infrastructure state during host sync.
    async fn on_host_synced(
        &self,
        ctx: &crate::agent_infra::InfraPluginContext<'_>,
        executor: &dyn uptrakit_command::RemoteExecutor,
        host_id: uuid::Uuid,
    ) -> Result<crate::agent_infra::SyncInfraResult>;

    /// Human-readable preview of the steps performed by [`on_host_synced`].
    ///
    /// Shown in the sync plan before execution. Return concise present-tense
    /// descriptions, e.g. `"Detect PVE node name"`. Default: empty.
    fn sync_step_previews(&self) -> Vec<String> {
        vec![]
    }

    /// Security impact level of [`on_host_synced`] for display in the sync plan.
    ///
    /// Default: [`Severity::Medium`].
    fn sync_security_impact(&self) -> uptrakit_shared_types::Severity {
        uptrakit_shared_types::Severity::Medium
    }

    /// Called after `ReportHosts` has been sent to the controller.
    async fn on_post_report_hosts(
        &self,
        ctx: &crate::agent_infra::InfraPluginContext<'_>,
    ) -> Result<()>;

    /// Called when the controller responds to a `ReportPluginConfig` request.
    async fn on_plugin_config_reported(
        &self,
        db: &sea_orm::DatabaseConnection,
        plugin_config_id: uuid::Uuid,
        request_id: &str,
    ) -> Result<()>;
}

/// Host state reporting for infrastructure plugins.
#[cfg(feature = "agent-infra")]
#[async_trait]
pub trait HostReport: PluginMeta {
    /// Check whether this plugin has infrastructure state for the given host.
    async fn has_infra_state(&self, db: &sea_orm::DatabaseConnection, host_id: uuid::Uuid) -> bool;
}

/// Guest execution capabilities for infrastructure plugins.
#[cfg(feature = "agent-infra")]
#[async_trait]
pub trait GuestExec: PluginMeta {
    /// Return a `GuestExecProvider` for executing commands inside guests.
    fn guest_exec_provider(&self) -> Option<Arc<dyn crate::agent_infra::GuestExecProvider>>;

    /// Handle a service-side surface action request.
    async fn handle_service_extension_action(
        &self,
        ctx: &crate::agent_infra::InfraPluginContext<'_>,
        request: &SurfaceActionRequest,
    ) -> Option<SurfaceActionResponse>;
}

#[cfg(test)]
mod execute_update_result_tests {
    use super::*;

    #[test]
    fn test_execute_update_result_default_not_resumable() {
        let r = ExecuteUpdateResult {
            output: "ok".to_string(),
            resumable: false,
        };
        assert!(!r.resumable);
    }
}

#[cfg(test)]
mod controller_boundary_tests {
    use super::*;

    struct TestNotificationStore;
    struct TestTelegramStore;
    struct TestProxmoxSurfaceStore;
    struct TestProxmoxStore;

    #[async_trait]
    impl NotificationChannelStore for TestNotificationStore {
        async fn list_channels(
            &self,
            req: NotificationChannelListRequest<'_>,
        ) -> Result<NotificationChannelListPage> {
            Ok(NotificationChannelListPage {
                items: vec![NotificationChannelListItem {
                    id: Uuid::new_v4(),
                    name: "primary".to_string(),
                    enabled: true,
                    created_at_rfc3339: "2026-01-01T00:00:00Z".to_string(),
                    config: serde_json::json!({"token": "***"}),
                }],
                total: 1,
                page: req.page,
                per_page: req.per_page,
                total_pages: 1,
            })
        }

        async fn resolve_action_token(
            &self,
            action_token: Uuid,
        ) -> Result<Option<NotificationActionTokenRecord>> {
            Ok(Some(NotificationActionTokenRecord {
                action_token,
                action_taken: None,
            }))
        }

        async fn mark_action_token_triggered(&self, _action_token: Uuid) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ProxmoxProtectionStore for TestProxmoxStore {
        async fn load_host_mapping(
            &self,
            _tenant_id: Uuid,
            _host_id: Uuid,
        ) -> Result<Option<ProxmoxHostMappingRecord>> {
            Ok(None)
        }

        async fn load_plugin_config_payload(
            &self,
            _tenant_id: Uuid,
            _plugin_config_id: Uuid,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }

        async fn load_effective_policy(
            &self,
            _tenant_id: Uuid,
            _software_item_id: Uuid,
            _plugin_config_id: Uuid,
        ) -> Result<ProxmoxProtectionPolicyRecord> {
            Ok(ProxmoxProtectionPolicyRecord::default())
        }

        async fn load_audit(
            &self,
            _update_history_id: Uuid,
        ) -> Result<Option<ProxmoxProtectionAuditRecord>> {
            Ok(None)
        }

        async fn upsert_audit(&self, _audit: &ProxmoxProtectionAuditRecord) -> Result<()> {
            Ok(())
        }

        async fn find_cached_backup_target(
            &self,
            _plugin_config_id: Uuid,
            _target_key: &str,
        ) -> Result<Option<String>> {
            Ok(None)
        }
    }

    #[async_trait]
    impl TelegramGlobalSettingsStore for TestTelegramStore {
        async fn load_global_bot_token(&self) -> Result<String> {
            Ok("token".to_string())
        }

        async fn save_global_bot_token(&self, bot_token: String) -> Result<String> {
            Ok(bot_token)
        }
    }

    #[async_trait]
    impl ProxmoxSurfaceStore for TestProxmoxSurfaceStore {
        async fn list_host_mappings(
            &self,
            _request: ProxmoxHostMappingsRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "items": [] }))
        }

        async fn discover_hosts(
            &self,
            _request: ProxmoxPluginConfigRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "discovered": 0 }))
        }

        async fn test_connection(
            &self,
            _request: ProxmoxPluginConfigRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "success": true }))
        }

        async fn match_host(
            &self,
            _request: ProxmoxManualMatchRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "success": true }))
        }

        async fn approve_match(
            &self,
            _request: ProxmoxApproveMatchRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "success": true }))
        }

        async fn unmatch_host(&self, _request: ProxmoxMappingRequest) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "success": true }))
        }

        async fn list_all_unmatched(
            &self,
            _request: ProxmoxUnmatchedGuestsRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "items": [] }))
        }

        async fn get_host_info(
            &self,
            _request: ProxmoxHostInfoRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "linked": false }))
        }

        async fn preload_global_defaults(
            &self,
            _request: ProxmoxScopeSelectionRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "mode": "do_nothing" }))
        }

        async fn save_global_defaults(
            &self,
            _request: ProxmoxGlobalDefaultsSaveRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "success": true }))
        }

        async fn preload_item_overrides(
            &self,
            _request: ProxmoxItemOverridePreloadRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "mode": "inherit_global" }))
        }

        async fn save_item_overrides(
            &self,
            _request: ProxmoxItemOverrideSaveRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "success": true }))
        }

        async fn load_backup_target_options(
            &self,
            _request: ProxmoxScopeSelectionRequest,
        ) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "options": [] }))
        }
    }

    struct TestController {
        tenant_id: Uuid,
        user_id: Option<Uuid>,
        notification_store: TestNotificationStore,
        telegram_store: TestTelegramStore,
        proxmox_surface_store: TestProxmoxSurfaceStore,
        proxmox_store: TestProxmoxStore,
    }

    impl SurfaceActionController for TestController {
        fn tenant_id(&self) -> Uuid {
            self.tenant_id
        }

        fn user_id(&self) -> Option<Uuid> {
            self.user_id
        }

        #[expect(
            clippy::unimplemented,
            reason = "tenant_db is never called by these unit tests"
        )]
        fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
            unimplemented!("tenant_db not used in roles.rs surface action tests")
        }

        fn notification_channel_store(&self) -> Option<&dyn NotificationChannelStore> {
            Some(&self.notification_store)
        }

        fn telegram_global_settings_store(&self) -> Option<&dyn TelegramGlobalSettingsStore> {
            Some(&self.telegram_store)
        }

        fn proxmox_surface_store(&self) -> Option<&dyn ProxmoxSurfaceStore> {
            Some(&self.proxmox_surface_store)
        }

        fn proxmox_protection_store(&self) -> Option<&dyn ProxmoxProtectionStore> {
            Some(&self.proxmox_store)
        }
    }

    impl UpdateProtectionController for TestController {
        #[expect(
            clippy::unimplemented,
            reason = "tenant_db is never called by these unit tests"
        )]
        fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
            unimplemented!("tenant_db not used in roles.rs protection tests")
        }

        fn proxmox_protection_store(&self) -> Option<&dyn ProxmoxProtectionStore> {
            Some(&self.proxmox_store)
        }
    }

    #[tokio::test]
    async fn notification_channel_store_lists_channels() {
        let store = TestNotificationStore;
        let page = store
            .list_channels(NotificationChannelListRequest {
                tenant_id: Uuid::new_v4(),
                channel_type: "email",
                page: 2,
                per_page: 10,
            })
            .await
            .expect("list should succeed");

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.page, 2);
        assert_eq!(page.per_page, 10);
    }

    #[tokio::test]
    async fn controller_capabilities_expose_first_wave_stores() {
        let controller = TestController {
            tenant_id: Uuid::new_v4(),
            user_id: Some(Uuid::new_v4()),
            notification_store: TestNotificationStore,
            telegram_store: TestTelegramStore,
            proxmox_surface_store: TestProxmoxSurfaceStore,
            proxmox_store: TestProxmoxStore,
        };

        assert!(controller.notification_channel_store().is_some());
        assert!(controller.telegram_global_settings_store().is_some());
        assert!(controller.proxmox_surface_store().is_some());
        assert!(SurfaceActionController::proxmox_protection_store(&controller).is_some());
        assert!(UpdateProtectionController::proxmox_protection_store(&controller).is_some());
    }
}
