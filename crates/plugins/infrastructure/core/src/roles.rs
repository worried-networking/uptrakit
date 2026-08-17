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
#[cfg(feature = "catalog")]
use crate::descriptor::GlobalProviderLookup;
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

/// Context provided to [`ReleaseFetcher`] factory functions at construction time.
///
/// Passed as the third argument alongside the config JSON and `HostRuntime`
/// when the scheduler creates a fetcher instance. Existing plugins ignore this
/// context; the `package-manager.skills` plugin reads `global_provider_lookup`
/// to reach the GitHub provider.
#[non_exhaustive]
pub struct ReleaseFetchContext {
    /// Global GitHub provider lookup, available when the embedded scheduler
    /// runs inside the controller. `None` in standalone-scheduler deployments.
    #[cfg(feature = "catalog")]
    pub global_provider_lookup: Option<std::sync::Arc<dyn GlobalProviderLookup>>,
}

impl ReleaseFetchContext {
    /// Construct a context with no provider lookup (standalone / test path).
    pub fn none() -> Self {
        Self {
            #[cfg(feature = "catalog")]
            global_provider_lookup: None,
        }
    }

    /// Construct from an `Option<Arc<dyn GlobalProviderLookup>>`.
    ///
    /// `None` → standalone / test path; `Some(lookup)` → controller path.
    #[cfg(feature = "catalog")]
    pub fn with_lookup_opt(lookup: Option<std::sync::Arc<dyn GlobalProviderLookup>>) -> Self {
        Self {
            global_provider_lookup: lookup,
        }
    }
}

impl Default for ReleaseFetchContext {
    fn default() -> Self {
        Self::none()
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

/// Per-item input to an [`InstalledVersionEnricher`].
#[non_exhaustive]
pub struct InstalledVersionItem {
    pub package_identifier: String,
    pub installed_version: Option<String>,
}

impl InstalledVersionItem {
    pub fn new(package_identifier: String, installed_version: Option<String>) -> Self {
        Self {
            package_identifier,
            installed_version,
        }
    }
}

/// Per-item output from an [`InstalledVersionEnricher`].
///
/// The dispatcher zips inputs ↔ outputs **by index** (the returned `Vec` MUST
/// be the same length and order as the input slice). `installed_version_echo`
/// doubles as a contract sanity-check — see the trait doc.
#[non_exhaustive]
pub struct InstalledVersionDisplay {
    pub package_identifier: String,
    pub installed_version_echo: Option<String>,
    pub display_version: Option<String>,
}

impl InstalledVersionDisplay {
    pub fn new(
        package_identifier: String,
        installed_version_echo: Option<String>,
        display_version: Option<String>,
    ) -> Self {
        Self {
            package_identifier,
            installed_version_echo,
            display_version,
        }
    }
}

/// Controller-only role: derive a human-friendly `installed_display_version`
/// from the raw `installed_version` an agent reported. Used when the raw value
/// is opaque (e.g. a git tree SHA) and the display string must come from
/// upstream metadata only the controller can reach.
#[async_trait]
pub trait InstalledVersionEnricher: PluginMeta {
    /// Returns a `Vec` of the same length and order as `items`. The
    /// dispatcher zips by index, not by `package_identifier`, so two items
    /// sharing a `package_identifier` (e.g. the same Skill installed on two
    /// hosts with different SHAs) stay distinct. Implementors MUST preserve
    /// order; the dispatcher checks length and treats a mismatch as a fatal
    /// contract violation (warn + drop all display values to `None`).
    ///
    /// **`None`-input contract**: if `items[i].installed_version` is `None`,
    /// implementors MUST return `display_version = None` for that index.
    /// Returning a phantom display for an unknown installed SHA is a
    /// trait-contract violation.
    async fn enrich_installed_versions(
        &self,
        items: &[InstalledVersionItem],
    ) -> Result<Vec<InstalledVersionDisplay>>;
}

/// Context object passed to [`InstalledVersionEnricher`] factories at construction
/// time. Mirrors [`ReleaseFetchContext`] (ADR-0015). Carries the optional
/// `GlobalProviderLookup` so Skills can reach the GitHub provider client.
#[non_exhaustive]
pub struct InstalledVersionEnrichmentContext {
    /// Global GitHub provider lookup, available when the embedded scheduler
    /// runs inside the controller. `None` in standalone-scheduler deployments.
    #[cfg(feature = "catalog")]
    pub global_provider_lookup: Option<std::sync::Arc<dyn GlobalProviderLookup>>,
}

impl InstalledVersionEnrichmentContext {
    /// Construct an empty context (no provider lookup). Available under all feature
    /// combinations; non-catalog builds use this exclusively.
    pub const fn empty() -> Self {
        Self {
            #[cfg(feature = "catalog")]
            global_provider_lookup: None,
        }
    }

    /// Attach a `GlobalProviderLookup` (builder method). Available only with `catalog`.
    /// Call sites that need conditional attachment wrap this in a **single positive**
    /// `#[cfg(feature = "catalog")]` block — never `#[cfg(not(...))]`. Example:
    ///
    /// ```ignore
    /// let mut ctx = InstalledVersionEnrichmentContext::empty();
    /// #[cfg(feature = "catalog")]
    /// {
    ///     ctx = ctx.with_lookup(provider_lookup);
    /// }
    /// ```
    #[cfg(feature = "catalog")]
    pub fn with_lookup(mut self, lookup: std::sync::Arc<dyn GlobalProviderLookup>) -> Self {
        self.global_provider_lookup = Some(lookup);
        self
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
    ///
    /// Delivery is all-or-nothing from the controller's perspective: the
    /// notification loop maps `Ok(())` to a `delivered` log row and any `Err`
    /// to a `failed` row — there is no partial-success state. A multi-recipient
    /// transport (e.g. email) that partially fails MUST therefore return `Err`
    /// listing every failed recipient (see
    /// `NotificationPluginError::RecipientsFailed`), never silently drop some.
    async fn deliver(
        &self,
        config: &serde_json::Value,
        settings: &serde_json::Value,
        message: &uptrakit_notification_plugin_core::DeliveryMessage,
    ) -> uptrakit_notification_plugin_core::Result<()>;

    /// Handles an inbound provider callback (e.g. Telegram Bot API webhook).
    /// This is NOT a surface interaction: it is invoked unauthenticated by
    /// external services through the public notification-callback route, with
    /// channel-specific verification inside the plugin (ADR-0028 / spec D2a).
    async fn handle_callback(
        &self,
        ctx: &crate::descriptor::SurfaceActionContext<'_>,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, crate::descriptor::SurfaceActionError> {
        let _ = (ctx, params);
        Err(crate::descriptor::SurfaceActionError::InvalidInput(
            format!(
                "callback not supported for channel type '{}'",
                self.plugin_type_id()
            ),
        ))
    }
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
}

/// Typed controller boundary for pre/post update protection workflows.
pub trait UpdateProtectionController: Send + Sync {
    /// Tenant-scoped database access for the update protection workflow.
    #[cfg(feature = "plugin-ops")]
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
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

/// Typed controller boundary for update hook workflows.
#[cfg(feature = "plugin-ops")]
pub trait UpdateHookController: Send + Sync {
    /// Tenant-scoped database access for the update hook workflow.
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
}

/// Context provided to the pre-update hook.
#[cfg(feature = "plugin-ops")]
#[non_exhaustive]
pub struct UpdateHookPreContext<'a> {
    pub controller: &'a dyn UpdateHookController,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    pub output_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
}

#[cfg(feature = "plugin-ops")]
impl<'a> UpdateHookPreContext<'a> {
    pub fn new(
        controller: &'a dyn UpdateHookController,
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

    pub fn with_output_tx(mut self, tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>) -> Self {
        self.output_tx = Some(tx);
        self
    }
}

/// Context provided to the post-update hook.
#[cfg(feature = "plugin-ops")]
#[non_exhaustive]
pub struct UpdateHookPostContext<'a> {
    pub controller: &'a dyn UpdateHookController,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub update_history_id: Uuid,
    pub final_status: uptrakit_shared_types::UpdateStatus,
    pub notification_ops: &'a dyn crate::plugin_ops::NotificationOps,
    /// Tenant-scoped DB handle required by `NotificationOps::send_transactional_email`.
    pub tenant_db: uptrakit_tenant_db::TenantDb,
}

#[cfg(feature = "plugin-ops")]
impl<'a> UpdateHookPostContext<'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "spec-defined context type; all fields are distinct semantic roles"
    )]
    pub fn new(
        controller: &'a dyn UpdateHookController,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        update_history_id: Uuid,
        final_status: uptrakit_shared_types::UpdateStatus,
        notification_ops: &'a dyn crate::plugin_ops::NotificationOps,
        tenant_db: uptrakit_tenant_db::TenantDb,
    ) -> Self {
        Self {
            controller,
            tenant_id,
            host_id,
            software_item_id,
            update_history_id,
            final_status,
            notification_ops,
            tenant_db,
        }
    }
}

/// Controller-side pre/post-update hook plugin (e.g. resource scaling).
///
/// Singleton created at catalog construction.
#[cfg(feature = "plugin-ops")]
#[async_trait]
pub trait ControllerUpdateHook: PluginMeta + Send + Sync {
    /// Called before update execution. Best-effort: returns `()` so that
    /// scale-up failure cannot accidentally block the Update.
    async fn prepare_pre_update_hook(&self, ctx: &UpdateHookPreContext<'_>);

    /// Called after update completion. Returns `Result<()>` so restore
    /// failures propagate to the dispatch wrapper for logging.
    async fn finalize_post_update_hook(
        &self,
        ctx: &UpdateHookPostContext<'_>,
    ) -> crate::error::Result<()>;
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
    /// Read-only infrastructure probe for the bootstrap connect phase.
    ///
    /// MUST NOT mutate the remote host — detection commands only. The execute
    /// phase re-runs detection itself; this result feeds the review step.
    /// Required (no default): a plugin that provisions in
    /// `on_host_bootstrapped` but is invisible at probe time would recreate
    /// the connect/execute disagreement this hook exists to prevent.
    async fn probe_host(
        &self,
        ctx: &crate::agent_infra::InfraPluginContext<'_>,
        executor: &dyn uptrakit_command::RemoteExecutor,
        host_id: uuid::Uuid,
        host_name: &str,
    ) -> Result<crate::agent_infra::InfraProbeResult>;

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
    ///
    /// `host_id` is the host whose report this ack confirms — correlated by
    /// the runtime's pending-ack map; implementations write per-host state,
    /// never scan positionally.
    async fn on_plugin_config_reported(
        &self,
        db: &sea_orm::DatabaseConnection,
        plugin_config_id: uuid::Uuid,
        host_id: uuid::Uuid,
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
mod release_fetch_context_tests {
    #[cfg(feature = "catalog")]
    use super::*;

    #[cfg(feature = "catalog")]
    #[test]
    fn release_fetch_context_none_has_no_lookup() {
        let ctx = ReleaseFetchContext::none();
        assert!(ctx.global_provider_lookup.is_none());
    }

    #[cfg(feature = "catalog")]
    #[test]
    fn release_fetch_context_default_equals_none() {
        // Default is a convenience alias for none().
        let ctx = ReleaseFetchContext::default();
        assert!(ctx.global_provider_lookup.is_none());
    }

    #[cfg(feature = "catalog")]
    #[test]
    fn release_fetch_context_with_lookup_opt_some_roundtrips_lookup() {
        use crate::descriptor::GlobalProviderLookup;
        use std::sync::Arc;

        struct DummyLookup;
        impl GlobalProviderLookup for DummyLookup {
            fn lookup(&self, _id: &str) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
                None
            }
        }

        let lookup: Arc<dyn GlobalProviderLookup> = Arc::new(DummyLookup);
        let ctx = ReleaseFetchContext::with_lookup_opt(Some(Arc::clone(&lookup)));
        assert!(
            ctx.global_provider_lookup.is_some(),
            "with_lookup_opt(Some(...)) must preserve the lookup in the context"
        );
    }

    #[cfg(feature = "catalog")]
    #[test]
    fn release_fetch_context_with_lookup_opt_none_is_none() {
        let ctx = ReleaseFetchContext::with_lookup_opt(None);
        assert!(ctx.global_provider_lookup.is_none());
    }
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

    struct TestController {
        tenant_id: Uuid,
        user_id: Option<Uuid>,
    }

    impl SurfaceActionController for TestController {
        fn tenant_id(&self) -> Uuid {
            self.tenant_id
        }

        fn user_id(&self) -> Option<Uuid> {
            self.user_id
        }

        #[cfg(feature = "plugin-ops")]
        #[expect(
            clippy::unimplemented,
            reason = "tenant_db is never called by these unit tests"
        )]
        fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
            unimplemented!("tenant_db not used in roles.rs surface action tests")
        }
    }

    impl UpdateProtectionController for TestController {
        #[cfg(feature = "plugin-ops")]
        #[expect(
            clippy::unimplemented,
            reason = "tenant_db is never called by these unit tests"
        )]
        fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
            unimplemented!("tenant_db not used in roles.rs protection tests")
        }
    }

    #[test]
    fn controller_boundary_traits_are_object_safe() {
        let controller = TestController {
            tenant_id: Uuid::new_v4(),
            user_id: Some(Uuid::new_v4()),
        };

        // Verify that both traits can be used as trait objects without
        // requiring any Proxmox-specific capability injection.
        let _surface: &dyn SurfaceActionController = &controller;
        let _protection: &dyn UpdateProtectionController = &controller;
    }

    #[cfg(feature = "plugin-ops")]
    #[test]
    fn update_hook_controller_trait_is_object_safe() {
        struct TestHookCtrl;
        impl UpdateHookController for TestHookCtrl {
            #[expect(
                clippy::unimplemented,
                reason = "stub method body never executes; test only checks trait-object coercion"
            )]
            fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb {
                unimplemented!("tenant_db not used in roles.rs hook controller tests")
            }
        }
        let ctrl = TestHookCtrl;
        let _dyn: &dyn UpdateHookController = &ctrl;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn installed_version_enrichment_context_empty() {
        let ctx = crate::InstalledVersionEnrichmentContext::empty();
        #[cfg(feature = "catalog")]
        assert!(ctx.global_provider_lookup.is_none());
        let _ = ctx;
    }

    #[tokio::test]
    async fn installed_version_enricher_trait_is_object_safe() {
        use std::sync::Arc;
        struct Noop;
        impl crate::PluginMeta for Noop {
            fn plugin_type_id(&self) -> crate::PluginTypeId {
                crate::PluginTypeId::new("test_noop_enricher")
            }
        }
        #[async_trait::async_trait]
        impl crate::InstalledVersionEnricher for Noop {
            async fn enrich_installed_versions(
                &self,
                items: &[crate::InstalledVersionItem],
            ) -> crate::Result<Vec<crate::InstalledVersionDisplay>> {
                Ok(items
                    .iter()
                    .map(|i| crate::InstalledVersionDisplay {
                        package_identifier: i.package_identifier.clone(),
                        installed_version_echo: i.installed_version.clone(),
                        display_version: None,
                    })
                    .collect())
            }
        }
        let arc: Arc<dyn crate::InstalledVersionEnricher> = Arc::new(Noop);
        let items = vec![crate::InstalledVersionItem {
            package_identifier: "x".to_string(),
            installed_version: Some("sha".to_string()),
        }];
        let out = arc.enrich_installed_versions(&items).await.expect("ok");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].installed_version_echo.as_deref(), Some("sha"));
        assert!(out[0].display_version.is_none());
    }
}
