//! Plugin descriptor and catalog configuration types.
//!
//! [`PluginDescriptor`] is the static metadata struct that every plugin exports.
//! [`CatalogConfig`] provides shared resources for singleton construction.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use uptrakit_extension_framework::{ActionDef, ExtensionManifest, FieldDef};
use uptrakit_internal_wire::ConfigTestKind;
use uptrakit_shared_types::PluginCapability;

use crate::host_requirements::HostRequirements;
use crate::host_runtime::HostRuntime;
use crate::roles;
use crate::traits::SudoCommandEntry;

/// Which family a plugin belongs to. Determines UI grouping and catalog queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginFamily {
    Software,
    Hook,
    Notification,
    Infrastructure,
    Enhancement,
}

impl std::fmt::Display for PluginFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Software => write!(f, "software"),
            Self::Hook => write!(f, "hook"),
            Self::Notification => write!(f, "notification"),
            Self::Infrastructure => write!(f, "infrastructure"),
            Self::Enhancement => write!(f, "enhancement"),
        }
    }
}

/// How a plugin's configuration is stored and validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigModel {
    /// Standard plugin config (stored in `plugin_configs` table).
    PluginConfig,
    /// Notification channel config (stored in `notification_channels` table).
    NotificationChannel,
    /// No per-instance config (enhancement plugins, discovery-only plugins).
    None,
}

// ── Config operations ───────────────────────────────────────────────────────

/// Config operations — every plugin has these. Grouped to keep the descriptor flat.
pub struct ConfigOps {
    pub validate: fn(&serde_json::Value) -> Result<(), String>,
    pub mask_secrets: fn(&serde_json::Value) -> serde_json::Value,
    pub restore_secrets: fn(&mut serde_json::Value, &serde_json::Value),
    pub sample: fn() -> serde_json::Value,
    pub form_schema: fn() -> Vec<FieldDef>,
    pub validate_identifier: fn(&str) -> Result<(), String>,
}

// ── Type settings ───────────────────────────────────────────────────────────

/// Type-level settings — only for package managers with tenant-scoped settings.
pub struct TypeSettingsOps {
    pub form_schema: fn() -> Vec<FieldDef>,
    pub sample: fn() -> serde_json::Value,
}

// ── Config test metadata ────────────────────────────────────────────────────

/// Config test metadata — which `ConfigTestKind`s a plugin supports and the default.
///
/// Only present when the plugin declares `config_test: [...]` in `declare_plugin!`.
pub struct ConfigTestOps {
    /// Which test kinds this plugin can handle. Non-empty.
    pub supported_kinds: &'static [ConfigTestKind],
    /// Default when the caller doesn't specify a kind. Must be in `supported_kinds`.
    pub default_kind: ConfigTestKind,
}

// ── Extension action context ────────────────────────────────────────────────

/// Context passed to plugin extension action handlers.
///
/// Provides access to the database connection and tenant/user context
/// from the authenticated HTTP request.
///
/// This struct is always compiled (not feature-gated) so that the
/// `ExtensionActionHandler` type alias is available in all plugin crates.
/// The `db` field uses `dyn std::any::Any` to avoid requiring `sea-orm`
/// at the type level — handler implementations downcast to `DatabaseConnection`.
pub struct ExtensionActionContext<'a> {
    /// Database connection (downcast to `sea_orm::DatabaseConnection`).
    pub db: &'a dyn std::any::Any,
    /// Tenant ID from the authenticated request (if available).
    pub tenant_id: Option<uuid::Uuid>,
    /// User ID of the caller, for actions that need it (e.g. sending test emails).
    pub caller_user_id: Option<uuid::Uuid>,
}

// ── Extension operations ────────────────────────────────────────────────────

/// Extension handling — only for plugins that own extension IDs.
pub struct ExtensionOps {
    pub manifests: fn() -> Vec<ExtensionManifest>,
    pub actions: fn() -> Vec<ActionDef>,
    pub owned_ids: &'static [&'static str],
    pub handle_action: ExtensionActionHandler,
}

// ── Type aliases ────────────────────────────────────────────────────────────

/// Sync creation for a software/hook role.
///
/// Receives `Arc<dyn HostRuntime>` — NOT `Arc<dyn CommandExecutor>`.
/// POSIX plugins extract the executor via `require_posix_executor()`.
pub type CreateRoleFn<R> =
    fn(&serde_json::Value, Arc<dyn HostRuntime>) -> crate::error::Result<Box<R>>;

/// Creation for a notification transport (singleton).
///
/// Always compiled — `CatalogConfig` is un-gated so this type is visible in all
/// plugin crates.
pub type CreateTransportFn =
    fn(&CatalogConfig) -> crate::error::Result<Arc<dyn roles::NotificationTransport>>;

/// Creation for an enhancement plugin (singleton).
///
/// Always compiled — same rationale as `CreateTransportFn`.
pub type CreateEnhancementFn =
    fn(&CatalogConfig) -> crate::error::Result<Arc<dyn roles::SoftwareItemLifecycle>>;

/// Async extension action handler.
pub type ExtensionActionHandler =
    for<'a> fn(
        &'a ExtensionActionContext<'a>,
        &'a str,           // extension_id
        &'a str,           // action_id
        serde_json::Value, // params
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send + 'a>>;

// ── Role slots ──────────────────────────────────────────────────────────────

/// A role creation function paired with its host requirements.
pub struct RoleSlot<R: ?Sized> {
    pub create: CreateRoleFn<R>,
    pub host_requirements: HostRequirements,
}

/// Infrastructure role slot — creation + requirements + capabilities.
#[cfg(feature = "agent-infra")]
pub struct InfraSlot {
    pub create: fn(&CatalogConfig) -> crate::error::Result<InfraBundle>,
    pub host_requirements: HostRequirements,
    pub capabilities: &'static [PluginCapability],
}

/// Bundle of narrow infrastructure trait objects returned by `InfraSlot::create`.
#[cfg(feature = "agent-infra")]
pub struct InfraBundle {
    /// Host lifecycle management (bootstrap, sync, post-report).
    pub lifecycle: Option<Arc<dyn roles::HostLifecycle>>,
    /// Host state reporting.
    pub report: Option<Arc<dyn roles::HostReport>>,
    /// Guest execution.
    pub guest_exec: Option<Arc<dyn roles::GuestExec>>,
}

// ── Role creators ───────────────────────────────────────────────────────────

/// Role creation function pointers. Most are `None` for any given plugin.
pub struct RoleCreators {
    // Per-instance roles (config + runtime → Box<dyn Role>)
    pub discoverer: Option<RoleSlot<dyn roles::Discoverer>>,
    pub version_detector: Option<RoleSlot<dyn roles::VersionDetector>>,
    pub release_fetcher: Option<RoleSlot<dyn roles::ReleaseFetcher>>,
    pub package_indexer: Option<RoleSlot<dyn roles::PackageIndexer>>,
    pub update_executor: Option<RoleSlot<dyn roles::UpdateExecutor>>,
    pub lifecycle_hook: Option<RoleSlot<dyn roles::LifecycleHook>>,
    // Singleton transport (catalog config → Arc, created once at startup)
    pub notification_transport: Option<CreateTransportFn>,
    // Singleton enhancement (catalog config → Arc, created once at startup)
    pub software_item_lifecycle: Option<CreateEnhancementFn>,
    // Singleton infra (catalog config → InfraBundle, created once per agent)
    #[cfg(feature = "agent-infra")]
    pub infra: Option<InfraSlot>,
}

// ── Plugin descriptor ───────────────────────────────────────────────────────

/// Static metadata struct exported by every plugin via `declare_plugin!`.
///
/// Groups fields to avoid becoming a dumping ground. Every plugin populates the
/// core fields and `config` / `roles` groups. Optional sections are only
/// populated by the plugins that use them.
pub struct PluginDescriptor {
    // ── Identity (every plugin) ──
    pub type_id: &'static str,
    pub display_name: &'static str,
    pub family: PluginFamily,
    pub config_model: ConfigModel,
    pub capabilities: &'static [PluginCapability],

    // ── Config operations (every plugin) ──
    pub config: ConfigOps,

    // ── Role creation (every plugin — most fields None) ──
    pub roles: RoleCreators,

    // ── Optional sections ──
    pub extensions: Option<&'static ExtensionOps>,
    pub type_settings: Option<&'static TypeSettingsOps>,
    pub config_test: Option<&'static ConfigTestOps>,
    /// Sudo commands required by this plugin.
    pub sudo: Option<fn(&serde_json::Value) -> Vec<SudoCommandEntry>>,
    pub raw_settings_keys: &'static [&'static str],

    // ── Migrations (feature-gated, controller-only) ──
    #[cfg(feature = "migrations")]
    #[allow(clippy::type_complexity)]
    pub migrations: Option<fn() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>>>,
}

impl PluginDescriptor {
    /// Look up a role slot or return a typed `PluginError::UnsupportedRole`.
    pub fn require_role_slot<'a, R: ?Sized>(
        &self,
        slot: Option<&'a RoleSlot<R>>,
        role_name: &str,
    ) -> crate::error::Result<&'a RoleSlot<R>> {
        slot.ok_or_else(|| {
            rootcause::report!(crate::error::PluginError::UnsupportedOperation(format!(
                "{} does not support role: {}",
                self.type_id, role_name
            )))
        })
    }
}

// ── CatalogConfig ───────────────────────────────────────────────────────────

/// Shared resources provided to `PluginCatalog::new()` for singleton construction.
///
/// **Always compiled** — NOT feature-gated. The struct itself is visible to all
/// plugin crates so they can reference it in `CreateTransportFn` / `CreateEnhancementFn`
/// type signatures.
///
/// Without `catalog`: struct has only `allow_private_urls: bool`.
/// With `catalog`: struct gains `http_client` and `cancellation_token` fields.
#[derive(Clone, Debug)]
pub struct CatalogConfig {
    /// When `true`, HTTP clients allow URLs pointing to private / loopback addresses.
    pub allow_private_urls: bool,
    /// Pre-configured base HTTP client (SSRF protection, timeouts).
    #[cfg(feature = "catalog")]
    pub http_client: Option<reqwest::Client>,
    /// Cancellation token for graceful shutdown of background tasks.
    #[cfg(feature = "catalog")]
    pub cancellation_token: Option<tokio_util::sync::CancellationToken>,
}

#[allow(clippy::derivable_impls)] // cfg-gated fields prevent derive
impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            allow_private_urls: false,
            #[cfg(feature = "catalog")]
            http_client: None,
            #[cfg(feature = "catalog")]
            cancellation_token: None,
        }
    }
}

// ── ControllerRuntime ───────────────────────────────────────────────────────

/// Runtime for controller-side plugins with no host.
///
/// Carries shared resources from `CatalogConfig`. Controller-side per-instance
/// roles (e.g., GitHub `ReleaseFetcher`) downcast to this to access the shared
/// HTTP client.
#[cfg(feature = "catalog")]
pub struct ControllerRuntime {
    config: CatalogConfig,
}

#[cfg(feature = "catalog")]
impl ControllerRuntime {
    pub fn new(config: CatalogConfig) -> Self {
        Self { config }
    }

    pub fn catalog_config(&self) -> &CatalogConfig {
        &self.config
    }

    pub fn http_client(&self) -> Option<&reqwest::Client> {
        self.config.http_client.as_ref()
    }

    pub fn cancellation_token(&self) -> Option<&tokio_util::sync::CancellationToken> {
        self.config.cancellation_token.as_ref()
    }
}

#[cfg(feature = "catalog")]
impl crate::host_runtime::HostRuntime for ControllerRuntime {
    fn capabilities(&self) -> &uptrakit_shared_types::HostCapabilities {
        static CAPS: std::sync::OnceLock<uptrakit_shared_types::HostCapabilities> =
            std::sync::OnceLock::new();
        CAPS.get_or_init(uptrakit_shared_types::HostCapabilities::default)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
