//! Plugin descriptor and catalog configuration types.
//!
//! [`PluginDescriptor`] is the static metadata struct that every plugin exports.
//! [`CatalogConfig`] provides shared resources for singleton construction.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use uptrakit_internal_wire::{ConfigTestKind, surfaces};
use uptrakit_shared_types::PluginCapability;

use crate::form_schema::FormFieldDescriptor;
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
    pub form_schema: fn() -> Vec<FormFieldDescriptor>,
    pub validate_identifier: fn(&str) -> Result<(), String>,
}

// ── Type settings ───────────────────────────────────────────────────────────

/// Type-level settings — only for package managers with tenant-scoped settings.
pub struct TypeSettingsOps {
    pub form_schema: fn() -> Vec<FormFieldDescriptor>,
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

// ── Global provider lookup ─────────────────────────────────────────────────

/// Opaque global provider handle lookup used by singleton/global plugin constructors.
pub trait GlobalProviderLookup: Send + Sync {
    /// Look up a provider handle by provider ID.
    fn lookup(&self, provider_id: &str) -> Option<Arc<dyn std::any::Any + Send + Sync>>;
}

/// Declarative consumer marker for global/shared provider handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalProviderConsumerDecl(pub &'static str);

impl GlobalProviderConsumerDecl {
    /// Global GitHub consumer used by the dashboard-icons enhancement.
    pub const DASHBOARD_ICONS: Self = Self("dashboard-icons");

    /// Borrow the consumer identifier as a string slice.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for GlobalProviderConsumerDecl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

// ── Surface action context ──────────────────────────────────────────────────

/// Context passed to plugin surface action handlers.
///
/// Provides access to the database connection and tenant/user context
/// from the authenticated HTTP request.
///
/// This struct is always compiled (not feature-gated) so that the
/// `SurfaceActionHandler` type alias is available in all plugin crates.
/// The `db` field uses `dyn std::any::Any` to avoid requiring `sea-orm`
/// at the type level — handler implementations downcast to `DatabaseConnection`.
pub struct SurfaceActionContext<'a> {
    /// Database connection (downcast to `sea_orm::DatabaseConnection`).
    pub db: &'a (dyn std::any::Any + Send + Sync),
    /// Tenant ID from the authenticated request (if available).
    pub tenant_id: Option<uuid::Uuid>,
    /// User ID of the caller, for actions that need it (e.g. sending test emails).
    pub caller_user_id: Option<uuid::Uuid>,
}

pub use crate::surface_form_authoring::{
    ApiSubmitDescriptor, SurfaceActionDescriptor, SurfaceActionUi, SurfaceFormDescriptor,
    SurfaceManifest, SurfacePlacement, SurfaceRowCondition, SurfaceRowVisibleWhen,
    SurfaceTableColumn, SurfaceTargeting, SurfaceUiDefinition, SurfaceWorkflowStep,
};

// ── Surface action library ──────────────────────────────────────────────────

/// Surface action library exported by plugins that expose controller-side surface actions.
pub struct SurfaceActionLibrary {
    pub actions: fn() -> Vec<SurfaceActionDescriptor>,
    pub owned_surface_ids: &'static [&'static str],
    pub handle_action: SurfaceActionHandler,
}

impl SurfaceActionLibrary {
    /// Surface-oriented accessor for owned route prefixes.
    pub fn owned_surface_ids(&self) -> &'static [&'static str] {
        self.owned_surface_ids
    }
}

/// Surface registration handling for plugin-backed compiled-in providers.
pub struct SurfaceRegistrationOps {
    pub registrations: fn() -> Vec<surfaces::SurfaceRegistration>,
}

// ── Type aliases ────────────────────────────────────────────────────────────

/// Sync creation for a software/hook role.
///
/// Receives `Arc<dyn HostRuntime>` — NOT `Arc<dyn CommandExecutor>`.
/// Plugins extract the executor via `runtime.executor()`.
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

/// Creation for a controller update protection plugin (singleton).
pub type CreateControllerProtectionFn =
    fn(&CatalogConfig) -> crate::error::Result<Arc<dyn roles::ControllerUpdateProtection>>;

/// Async surface action handler.
pub type SurfaceActionHandler =
    for<'a> fn(
        &'a SurfaceActionContext<'a>,
        &'a str,           // surface_id
        &'a str,           // action_id
        serde_json::Value, // params
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send + 'a>>;

/// Migrations function pointer type.
///
/// When `migrations` feature is active, this is a function that returns real
/// migration trait objects. When the feature is off, it's a simple function
/// returning an empty `Vec<Box<dyn Any>>` — the field exists but is meaningless.
#[cfg(feature = "migrations")]
pub type MigrationsFn = fn() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>>;

/// Placeholder type when `migrations` feature is not enabled.
#[cfg(not(feature = "migrations"))]
pub type MigrationsFn = fn() -> Vec<Box<dyn std::any::Any>>;

// ── Role slots ──────────────────────────────────────────────────────────────

/// A role creation function paired with its host requirements.
pub struct RoleSlot<R: ?Sized> {
    pub create: CreateRoleFn<R>,
    pub host_requirements: HostRequirements,
}

/// Infrastructure role slot — creation + requirements + capabilities.
///
/// Only meaningful when the `agent-infra` feature is active. When the feature
/// is off, `InfraSlot` is a zero-size placeholder so that `RoleCreators` and
/// `PluginDescriptor` have a stable layout regardless of feature flags. This
/// avoids struct-field mismatch when the `declare_plugin!` macro expands in
/// plugin crates that don't have `agent-infra` themselves but depend on
/// `plugin-infrastructure-core` which may be compiled with it.
#[cfg(feature = "agent-infra")]
pub struct InfraSlot {
    pub create: fn(&CatalogConfig) -> crate::error::Result<InfraBundle>,
    pub host_requirements: HostRequirements,
    pub capabilities: &'static [PluginCapability],
}

/// Placeholder when `agent-infra` is not enabled.
#[cfg(not(feature = "agent-infra"))]
pub struct InfraSlot {
    _private: (),
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

/// Placeholder when `agent-infra` is not enabled.
#[cfg(not(feature = "agent-infra"))]
pub struct InfraBundle {
    _private: (),
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
    // Singleton controller-side update protection (catalog config → Arc, created once at startup)
    pub controller_update_protection: Option<CreateControllerProtectionFn>,
    // Singleton infra (catalog config → InfraBundle, created once per agent).
    // Always present (not cfg-gated) so that `declare_plugin!` macro expansions
    // in consuming crates always see the field, regardless of feature flags.
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
    pub surface_actions: Option<&'static SurfaceActionLibrary>,
    pub surfaces: Option<&'static SurfaceRegistrationOps>,
    pub type_settings: Option<&'static TypeSettingsOps>,
    pub config_test: Option<&'static ConfigTestOps>,
    /// Sudo commands required by this plugin.
    pub sudo: Option<fn(&serde_json::Value) -> Vec<SudoCommandEntry>>,
    pub raw_settings_keys: &'static [&'static str],
    /// Global provider consumers declared by this plugin.
    pub global_provider_consumers: &'static [GlobalProviderConsumerDecl],

    // ── Migrations (controller-only) ──
    // Always present so `declare_plugin!` macro expansions always see the field.
    // The actual type is only meaningful when `migrations` feature is active.
    #[allow(clippy::type_complexity)]
    pub migrations: Option<MigrationsFn>,
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
#[derive(Clone)]
pub struct CatalogConfig {
    /// When `true`, HTTP clients allow URLs pointing to private / loopback addresses.
    pub allow_private_urls: bool,
    /// Host-owned lookup for global provider handles used by singleton plugins.
    pub global_provider_lookup: Option<Arc<dyn GlobalProviderLookup>>,
    /// Pre-configured base HTTP client (SSRF protection, timeouts).
    #[cfg(feature = "catalog")]
    pub http_client: Option<reqwest::Client>,
    /// Cancellation token for graceful shutdown of background tasks.
    #[cfg(feature = "catalog")]
    pub cancellation_token: Option<tokio_util::sync::CancellationToken>,
}

impl std::fmt::Debug for CatalogConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut ds = f.debug_struct("CatalogConfig");
        ds.field("allow_private_urls", &self.allow_private_urls)
            .field(
                "global_provider_lookup",
                &self.global_provider_lookup.as_ref().map(|_| "set"),
            );
        #[cfg(feature = "catalog")]
        {
            ds.field("http_client", &self.http_client.as_ref().map(|_| "set"))
                .field(
                    "cancellation_token",
                    &self.cancellation_token.as_ref().map(|_| "set"),
                );
        }
        ds.finish()
    }
}

#[allow(clippy::derivable_impls)] // cfg-gated fields prevent derive
impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            allow_private_urls: false,
            global_provider_lookup: None,
            #[cfg(feature = "catalog")]
            http_client: None,
            #[cfg(feature = "catalog")]
            cancellation_token: None,
        }
    }
}

// ── ControllerRuntime ───────────────────────────────────────────────────────

/// Runtime for controller-side plugins.
///
/// Wraps a [`StandardHostRuntime`] with [`LocalCommandExecutor`] for local
/// command execution. Carries shared resources from [`CatalogConfig`].
/// Controller-side per-instance roles (e.g., GitHub `ReleaseFetcher`) access
/// the executor via the [`HostRuntime::executor()`] trait method.
#[cfg(feature = "catalog")]
pub struct ControllerRuntime {
    local_runtime: std::sync::Arc<dyn crate::host_runtime::HostRuntime>,
    config: CatalogConfig,
}

#[cfg(feature = "catalog")]
impl ControllerRuntime {
    pub fn new(config: CatalogConfig) -> Self {
        let local_runtime: std::sync::Arc<dyn crate::host_runtime::HostRuntime> =
            std::sync::Arc::new(crate::host_runtime::StandardHostRuntime::new(
                std::sync::Arc::new(uptrakit_command::LocalCommandExecutor),
                uptrakit_shared_types::HostCapabilities::default(),
            ));
        Self {
            local_runtime,
            config,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(
        config: CatalogConfig,
        executor: std::sync::Arc<dyn uptrakit_command::CommandExecutor>,
    ) -> Self {
        let local_runtime: std::sync::Arc<dyn crate::host_runtime::HostRuntime> =
            std::sync::Arc::new(crate::host_runtime::StandardHostRuntime::new(
                executor,
                uptrakit_shared_types::HostCapabilities::default(),
            ));
        Self {
            local_runtime,
            config,
        }
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
        self.local_runtime.capabilities()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executor(&self) -> std::sync::Arc<dyn uptrakit_command::CommandExecutor> {
        self.local_runtime.executor()
    }
}

#[cfg(test)]
mod controller_runtime_tests {
    use super::*;
    use crate::host_runtime::HostRuntime;
    use uptrakit_command::NoopCommandExecutor;

    #[test]
    fn controller_runtime_provides_executor() {
        let rt = ControllerRuntime::new_for_test(
            CatalogConfig::default(),
            std::sync::Arc::new(NoopCommandExecutor),
        );
        let _exec = rt.executor();
    }

    #[test]
    fn controller_runtime_preserves_identity() {
        let rt = ControllerRuntime::new_for_test(
            CatalogConfig::default(),
            std::sync::Arc::new(NoopCommandExecutor),
        );
        let any = rt.as_any();
        assert!(
            any.downcast_ref::<ControllerRuntime>().is_some(),
            "as_any() should return ControllerRuntime, not the inner runtime"
        );
    }

    #[test]
    fn controller_runtime_catalog_config_accessible() {
        let rt = ControllerRuntime::new_for_test(
            CatalogConfig::default(),
            std::sync::Arc::new(NoopCommandExecutor),
        );
        let _config = rt.catalog_config();
    }

    #[test]
    fn production_new_provides_executor() {
        let rt = ControllerRuntime::new(CatalogConfig::default());
        let _exec = rt.executor();
    }
}
