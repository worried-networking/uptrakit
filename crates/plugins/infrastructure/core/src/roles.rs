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
use serde::de::DeserializeOwned;
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
use uptrakit_internal_wire::surfaces::{SurfaceActionRequest, SurfaceActionResponse};

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
    ) -> Result<String>;

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
                Ok(output) => results.push(BatchUpdateResult {
                    package_identifier: item.package_identifier.clone(),
                    success: true,
                    output,
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
    /// Database connection (downcast by implementations as needed).
    pub db: &'a (dyn std::any::Any + Send + Sync),
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
}

impl<'a> ControllerProtectionContext<'a> {
    /// Construct a controller pre-update protection context.
    pub fn new(
        db: &'a (dyn std::any::Any + Send + Sync),
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        update_history_id: Uuid,
    ) -> Self {
        Self {
            db,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
        }
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
    /// Database connection (downcast by implementations as needed).
    pub db: &'a (dyn std::any::Any + Send + Sync),
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    pub final_status: uptrakit_shared_types::UpdateStatus,
}

impl<'a> ControllerPostUpdateContext<'a> {
    /// Construct a controller post-update finalization context.
    pub fn new(
        db: &'a (dyn std::any::Any + Send + Sync),
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        update_history_id: Uuid,
        final_status: uptrakit_shared_types::UpdateStatus,
    ) -> Self {
        Self {
            db,
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
