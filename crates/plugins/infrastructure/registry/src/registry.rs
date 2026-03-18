use std::sync::Arc;

use rootcause::prelude::*;

use uptrakit_command::CommandExecutor;
use uptrakit_plugin_discovery_proxmox_helper_scripts::{
    ProxmoxHelperScriptsConfig, ProxmoxHelperScriptsPlugin,
};
use uptrakit_plugin_generic_shell::{ShellConfig, ShellPlugin};
use uptrakit_plugin_hook_shell::{ShellHookConfig, ShellHookPlugin};
use uptrakit_plugin_hook_systemd::{SystemdHookConfig, SystemdHookPlugin};
use uptrakit_plugin_infrastructure_core::{
    ConfigFormSchema, PluginBase, PluginType, SecretMasking, SudoCommandEntry,
};
use uptrakit_plugin_infrastructure_proxmox::{ProxmoxConfig, ProxmoxPlugin};
use uptrakit_plugin_package_manager_apk::{ApkConfig, ApkPlugin};
use uptrakit_plugin_package_manager_apt::{AptConfig, AptPlugin};
use uptrakit_plugin_package_manager_cargo::{CargoConfig, CargoPlugin};
use uptrakit_plugin_package_manager_dnf::{DnfConfig, DnfPlugin};
use uptrakit_plugin_package_manager_homebrew::{HomebrewConfig, HomebrewPlugin};
use uptrakit_plugin_package_manager_mas::{MasConfig, MasPlugin};
use uptrakit_plugin_package_manager_npm::{NpmConfig, NpmPlugin};
use uptrakit_plugin_package_manager_pacman::{PacmanConfig, PacmanPlugin};
use uptrakit_plugin_package_manager_pkg::{PkgConfig, PkgPlugin};
use uptrakit_plugin_package_manager_snap::{SnapConfig, SnapPlugin};
use uptrakit_plugin_releases_docker::{DockerConfig, DockerPlugin};
use uptrakit_plugin_releases_forgejo::{ForgejoConfig, ForgejoPlugin};
use uptrakit_plugin_releases_github::{GitHubConfig, GitHubPlugin};
use uptrakit_plugin_releases_gitlab::{GitLabConfig, GitLabPlugin};

#[cfg(feature = "notifications")]
use crate::NotificationRegistryConfig;
use crate::error::{PluginRegistryError, Result};

/// Deserialize, mask secrets via [`SecretMasking`], and re-serialize.
///
/// If re-serialization of the masked config fails (which should never happen
/// in practice), an error is logged and the **original unmasked config** is
/// returned. Callers must never silently discard such an outcome: the log
/// entry is the production signal that masking is broken.
fn mask_secrets_for<T: SecretMasking>(config: &serde_json::Value) -> serde_json::Value {
    let Ok(cfg) = serde_json::from_value::<T>(config.clone()) else {
        return config.clone();
    };
    match serde_json::to_value(cfg.with_secrets_masked()) {
        Ok(masked) => masked,
        Err(e) => {
            tracing::error!(
                error = %e,
                "failed to serialize masked plugin config; \
                 falling back to original — plugin secrets may be exposed in API responses"
            );
            config.clone()
        }
    }
}

/// Deserialize both values, restore secrets via [`SecretMasking`], and write back.
fn restore_secrets_for<T: SecretMasking>(
    incoming: &mut serde_json::Value,
    existing: &serde_json::Value,
) {
    let (Ok(mut inc), Ok(ex)) = (
        serde_json::from_value::<T>(incoming.clone()),
        serde_json::from_value::<T>(existing.clone()),
    ) else {
        return;
    };
    inc.restore_secrets_from(&ex);
    if let Ok(v) = serde_json::to_value(&inc) {
        *incoming = v;
    }
}

// ── Notification transport adapter (bridging new framework to legacy PluginBase) ─

/// Adapter bridging a migrated notification plugin (with `PluginDescriptor` +
/// `NotificationTransport`) to the legacy `PluginBase` interface.
///
/// During the migration, the registry still stores notification plugins as
/// `Arc<dyn PluginBase>`. Migrated plugins (starting with webhook) produce a
/// `PluginDescriptor` and an `Arc<dyn NotificationTransport>` but no longer
/// implement `PluginBase` directly. This adapter wraps both and delegates:
///
/// - Config ops (`validate_config`, `mask_config_secrets`, `restore_config_secrets`)
///   go through `descriptor.config.*` function pointers.
/// - `as_notification_transport()` returns a thin inner wrapper that adapts the
///   new `NotificationTransport` to the old `NotificationTransportPlugin`.
///
/// Once all notification plugins are migrated and the registry is refactored to
/// use `PluginDescriptor` directly, this adapter can be removed.
#[cfg(feature = "notifications")]
struct NotificationPluginAdapter {
    descriptor: &'static uptrakit_plugin_infrastructure_core::PluginDescriptor,
    transport: Arc<dyn uptrakit_plugin_infrastructure_core::NotificationTransport>,
}

#[cfg(feature = "notifications")]
#[async_trait::async_trait]
impl PluginBase for NotificationPluginAdapter {
    fn plugin_type_id(&self) -> &str {
        self.descriptor.type_id
    }

    fn capabilities(&self) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
        self.descriptor.capabilities.to_vec()
    }

    fn validate_config(&self, config: &serde_json::Value) -> std::result::Result<(), String> {
        (self.descriptor.config.validate)(config)
    }

    fn mask_config_secrets(&self, config: &serde_json::Value) -> serde_json::Value {
        (self.descriptor.config.mask_secrets)(config)
    }

    fn restore_config_secrets(
        &self,
        incoming: &serde_json::Value,
        stored: &serde_json::Value,
    ) -> serde_json::Value {
        let mut val = incoming.clone();
        (self.descriptor.config.restore_secrets)(&mut val, stored);
        val
    }

    fn as_notification_transport(
        &self,
    ) -> Option<&dyn uptrakit_plugin_infrastructure_core::NotificationTransportPlugin> {
        Some(self)
    }
}

/// Implement the old `NotificationTransportPlugin` on the adapter by
/// delegating to the wrapped `Arc<dyn NotificationTransport>`.
#[cfg(feature = "notifications")]
#[async_trait::async_trait]
impl uptrakit_plugin_infrastructure_core::NotificationTransportPlugin
    for NotificationPluginAdapter
{
    fn channel_type(&self) -> &'static str {
        self.descriptor.type_id
    }

    async fn deliver(
        &self,
        config: &serde_json::Value,
        settings: &serde_json::Value,
        message: &uptrakit_notification_plugin_core::DeliveryMessage,
    ) -> uptrakit_notification_plugin_core::Result<()> {
        self.transport.deliver(config, settings, message).await
    }
}

/// Probe whether a plugin instance is compatible with the target host.
///
/// Returns `true` when the plugin is compatible or when the compatibility
/// check is unavailable. Returns `false` only when the plugin explicitly
/// reports [`HostCompatibility::Incompatible`].
///
/// If a compatibility check returns an error, it is logged as a debug
/// message and the plugin is treated as compatible (fail-open).
async fn probe_plugin_host_compatibility(
    plugin: &dyn uptrakit_plugin_infrastructure_core::PluginBase,
    pt: uptrakit_plugin_infrastructure_core::PluginType,
) -> bool {
    if !plugin.has_capability(
        uptrakit_plugin_infrastructure_core::PluginCapability::DetectHostCompatibility,
    ) {
        return true;
    }
    let Some(discovery) = plugin.as_discovery() else {
        // Plugin declares DetectHostCompatibility but doesn't implement
        // DiscoveryPlugin — assume compatible.
        return true;
    };
    match discovery.detect_host_compatibility().await {
        Ok(uptrakit_plugin_infrastructure_core::HostCompatibility::Compatible) => true,
        Ok(uptrakit_plugin_infrastructure_core::HostCompatibility::Incompatible(reason)) => {
            tracing::debug!(
                plugin = %pt,
                reason = %reason,
                "plugin not compatible with host; skipping sudo commands"
            );
            false
        }
        Ok(_) => {
            tracing::warn!(
                plugin = %pt,
                "unknown HostCompatibility variant; assuming compatible"
            );
            true
        }
        Err(e) => {
            tracing::debug!(
                plugin = %pt,
                error = %e,
                "host compatibility check failed; assuming compatible"
            );
            true
        }
    }
}

/// Generates the `PluginRegistry` dispatch methods from a single
/// declaration list, eliminating manually-maintained match arms.
///
/// Each entry may optionally specify `extension_prefix: "prefix.", extension_handler: path::to::fn`
/// to participate in [`PluginRegistry::handle_extension_action`] dispatch.
/// When specified, any `extension_id` that starts with that prefix is
/// routed to the provided handler function.
macro_rules! register_plugins {
    ($(
        $variant:ident => {
            config: $config:ty,
            plugin: $plugin:ty
            $(, extension_prefix: $ext_prefix:literal, extension_handler: $ext_handler:path)?
        }
    ),+ $(,)?) => {
        impl PluginRegistry {
            /// Create a plugin instance from plugin type, config, and executor.
            ///
            /// Deserializes the config, validates it, and constructs the plugin.
            /// All plugins follow the same pattern: deserialize → validate →
            /// construct.
            #[tracing::instrument(skip_all)]
            pub async fn create_plugin(
                plugin_type: PluginType,
                config: &serde_json::Value,
                executor: Arc<dyn CommandExecutor>,
            ) -> Result<Box<dyn PluginBase>> {
                match plugin_type {
                    $(
                        PluginType::$variant => {
                            let typed_config: $config =
                                serde_json::from_value(config.clone()).context_to()?;
                            typed_config
                                .validate()
                                .map_err(|e| report!(PluginRegistryError::ConfigValidation(e.to_string())))?;
                            let plugin = <$plugin>::new(typed_config, executor).await
                                .map_err(|e| report!(PluginRegistryError::Instantiation(e.to_string())))?;
                            Ok(Box::new(plugin))
                        }
                    )+
                    _ => Err(report!(PluginRegistryError::UnknownPluginType(format!(
                        "{plugin_type}"
                    )))),
                }
            }

            /// Validate plugin configuration JSON.
            pub fn validate_config(
                plugin_type: PluginType,
                config: &serde_json::Value,
            ) -> Result<()> {
                match plugin_type {
                    $(
                        PluginType::$variant => {
                            let typed_config: $config =
                                serde_json::from_value(config.clone()).context_to()?;
                            typed_config
                                .validate()
                                .map_err(|e| report!(PluginRegistryError::ConfigValidation(e.to_string())))?;
                            Ok(())
                        }
                    )+
                    _ => Err(report!(PluginRegistryError::UnknownPluginType(format!(
                        "{plugin_type}"
                    )))),
                }
            }

            /// Mask secrets in plugin configuration JSON for API responses.
            ///
            /// Deserializes config, calls [`SecretMasking::with_secrets_masked()`],
            /// and serializes back. Unknown plugin types are returned unchanged.
            #[must_use]
            pub fn mask_config_secrets(
                plugin_type: PluginType,
                config: &serde_json::Value,
            ) -> serde_json::Value {
                match plugin_type {
                    $(
                        PluginType::$variant => mask_secrets_for::<$config>(config),
                    )+
                    _ => config.clone(),
                }
            }

            /// Restore masked secrets from existing configuration.
            ///
            /// Deserializes both incoming and existing configs, calls
            /// [`SecretMasking::restore_secrets_from()`], and writes back to
            /// `incoming`.
            pub fn restore_config_secrets(
                plugin_type: PluginType,
                incoming: &mut serde_json::Value,
                existing: &serde_json::Value,
            ) {
                match plugin_type {
                    $(
                        PluginType::$variant => {
                            restore_secrets_for::<$config>(incoming, existing);
                        }
                    )+
                    _ => {}
                }
            }

            /// Create a plugin instance for autodiscovery, bypassing `validate()`.
            ///
            /// Discovery can proceed with an empty/minimal config.  For plugins
            /// whose `validate()` is a no-op (e.g. `ProxmoxHelperScripts`) the two
            /// construction paths are equivalent.
            pub async fn create_plugin_for_discovery(
                plugin_type: PluginType,
                config: &serde_json::Value,
                executor: Arc<dyn CommandExecutor>,
            ) -> Result<Box<dyn PluginBase>> {
                match plugin_type {
                    $(
                        PluginType::$variant => {
                            let typed_config: $config =
                                serde_json::from_value(config.clone()).context_to()?;
                            // No validate() — discovery can proceed with an empty/minimal config.
                            let plugin = <$plugin>::new(typed_config, executor).await
                                .map_err(|e| report!(PluginRegistryError::Instantiation(e.to_string())))?;
                            Ok(Box::new(plugin))
                        }
                    )+
                    _ => Err(report!(PluginRegistryError::UnknownPluginType(format!(
                        "{plugin_type}"
                    )))),
                }
            }

            /// Returns all plugin types registered in the registry.
            ///
            /// This is the authoritative list of known plugin types. Use this
            /// instead of any hardcoded list outside the registry.
            pub fn known_plugin_types() -> Vec<PluginType> {
                vec![$(PluginType::$variant),+]
            }

            /// Returns a sample/default configuration JSON for the given plugin type.
            ///
            /// Serializes the `Default` implementation of the plugin's config type.
            /// Returns an empty JSON object `{}` for unknown / `Other` types.
            #[must_use]
            pub fn sample_config(plugin_type: PluginType) -> serde_json::Value {
                match plugin_type {
                    $(
                        PluginType::$variant => {
                            serde_json::to_value(<$config>::default())
                                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()))
                        }
                    )+
                    _ => serde_json::Value::Object(serde_json::Map::new()),
                }
            }

            /// Returns a sample/default configuration JSON for the given plugin type string.
            ///
            /// Returns an empty JSON object `{}` for unknown plugin types.
            #[must_use]
            pub fn sample_config_str(plugin_type: &str) -> serde_json::Value {
                let Ok(pt) = plugin_type.parse::<PluginType>() else {
                    return serde_json::Value::Object(serde_json::Map::new());
                };
                Self::sample_config(pt)
            }

            /// Returns form field definitions for the given plugin type.
            ///
            /// Uses the [`ConfigFormSchema`] trait implementation on each config
            /// type. Returns `None` for unknown / `Other` types.
            pub fn config_form_schema(
                plugin_type: PluginType,
            ) -> Option<Vec<uptrakit_extension_framework::FieldDef>> {
                match plugin_type {
                    $(
                        PluginType::$variant => Some(<$config as ConfigFormSchema>::form_schema()),
                    )+
                    _ => None,
                }
            }

            /// String-accepting convenience wrapper around [`config_form_schema`].
            pub fn config_form_schema_str(
                plugin_type: &str,
            ) -> Option<Vec<uptrakit_extension_framework::FieldDef>> {
                let Ok(pt) = plugin_type.parse::<PluginType>() else {
                    return None;
                };
                Self::config_form_schema(pt)
            }

            /// Returns type-settings form field definitions for the given plugin type.
            ///
            /// Uses [`ConfigFormSchema::type_settings_form_schema`]. Returns `None`
            /// for unknown / `Other` types.
            pub fn type_settings_form_schema(
                plugin_type: PluginType,
            ) -> Option<Vec<uptrakit_extension_framework::FieldDef>> {
                match plugin_type {
                    $(
                        PluginType::$variant => Some(<$config as ConfigFormSchema>::type_settings_form_schema()),
                    )+
                    _ => None,
                }
            }

            /// String-accepting convenience wrapper around [`type_settings_form_schema`].
            pub fn type_settings_form_schema_str(
                plugin_type: &str,
            ) -> Option<Vec<uptrakit_extension_framework::FieldDef>> {
                let Ok(pt) = plugin_type.parse::<PluginType>() else {
                    return None;
                };
                Self::type_settings_form_schema(pt)
            }

            /// Returns a sample/default JSON for type settings of the given plugin type.
            pub fn type_settings_sample(plugin_type: PluginType) -> serde_json::Value {
                match plugin_type {
                    $(
                        PluginType::$variant => {
                            <$config as ConfigFormSchema>::type_settings_sample()
                        }
                    )+
                    _ => serde_json::Value::Object(serde_json::Map::new()),
                }
            }

            /// String-accepting convenience wrapper around [`type_settings_sample`].
            #[must_use]
            pub fn type_settings_sample_str(plugin_type: &str) -> serde_json::Value {
                let Ok(pt) = plugin_type.parse::<PluginType>() else {
                    return serde_json::Value::Object(serde_json::Map::new());
                };
                Self::type_settings_sample(pt)
            }

            /// Returns all plugin types that have the `DiscoverLocalSoftware` capability.
            ///
            /// Uses compile-time `CAPABILITIES` constants — no instantiation needed.
            pub fn discovery_plugins() -> Vec<PluginType> {
                let mut result = Vec::new();
                $(
                    if <$plugin>::CAPABILITIES
                        .contains(&uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware)
                    {
                        result.push(PluginType::$variant);
                    }
                )+
                result
            }

            /// Returns the compile-time capabilities for a plugin type.
            ///
            /// Uses `CAPABILITIES` associated constants — no instantiation needed.
            /// Returns an empty slice for unknown / `Other` types.
            pub fn static_capabilities(
                plugin_type: PluginType,
            ) -> &'static [uptrakit_plugin_infrastructure_core::PluginCapability] {
                match plugin_type {
                    $(
                        PluginType::$variant => {
                            <$plugin>::CAPABILITIES
                        }
                    )+
                    _ => &[],
                }
            }

            /// Returns required sudo command entries for every registered plugin.
            ///
            /// Iterates all known plugin types, instantiates each with an empty
            /// config (using `create_plugin_for_discovery` which bypasses validation),
            /// calls `required_sudo_commands()`, and collects non-empty results.
            ///
            /// Used by the bootstrap process and `update-sudoers` command to generate
            /// minimal, per-command sudoers entries rather than a blanket
            /// `NOPASSWD: ALL` rule.
            pub async fn all_required_sudo_commands() -> Vec<(PluginType, Vec<SudoCommandEntry>)> {
                let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
                let empty = serde_json::Value::Object(serde_json::Map::new());
                let mut result = Vec::new();
                $(
                    if let Ok(p) = Self::create_plugin_for_discovery(
                        PluginType::$variant, &empty, executor.clone()).await
                    {
                        let entries = p.required_sudo_commands();
                        if !entries.is_empty() {
                            result.push((PluginType::$variant, entries));
                        }
                    }
                )+
                result
            }

            /// Validate a package identifier for the given plugin type.
            ///
            /// Dispatches to [`<$config>::validate_identifier`] for each registered
            /// plugin type. Returns `Ok(())` for plugin types that have no
            /// identifier constraints (including `Other`). Returns `Err(message)`
            /// when the identifier violates plugin-specific rules.
            pub fn validate_package_identifier(
                plugin_type: PluginType,
                value: &str,
            ) -> std::result::Result<(), String> {
                match plugin_type {
                    $(
                        PluginType::$variant => <$config>::validate_identifier(value),
                    )+
                    _ => Ok(()),
                }
            }

            /// Returns sudo command entries only for plugins that are compatible
            /// with the target host, as determined by running each plugin's
            /// [`DiscoveryPlugin::detect_host_compatibility`] check over the
            /// provided executor.
            ///
            /// Unlike [`all_required_sudo_commands`], this method runs host
            /// compatibility checks for every plugin that declares the
            /// [`PluginCapability::DetectHostCompatibility`] capability.  Plugins
            /// that report incompatibility (e.g. the Proxmox Helper Scripts plugin
            /// on a Flatcar Linux host where `/usr/bin/update` does not exist) are
            /// silently skipped so their helper scripts are never installed.
            ///
            /// Plugins that do **not** declare `DetectHostCompatibility` are always
            /// included — they are assumed compatible with all hosts.
            ///
            /// If a compatibility check returns an error it is logged as a warning
            /// and the plugin is treated as compatible (fail-open) to preserve
            /// existing behaviour on ambiguous targets.
            ///
            /// # Arguments
            ///
            /// * `executor` — A [`CommandExecutor`] connected to the target host
            ///   (typically an [`SshCommandExecutor`]).  Commands are executed on
            ///   the remote side so the compatibility check reflects the actual
            ///   host environment.
            pub async fn compatible_sudo_commands_for_host(
                executor: Arc<dyn CommandExecutor>,
            ) -> Vec<(PluginType, Vec<SudoCommandEntry>)> {
                use futures_util::future::join_all;
                let empty = serde_json::Value::Object(serde_json::Map::new());

                // Phase 1: create all plugin instances sequentially.
                let mut instances: Vec<(PluginType, Box<dyn PluginBase>)> = Vec::new();
                $(
                    if let Ok(p) = Self::create_plugin_for_discovery(
                        PluginType::$variant, &empty, Arc::clone(&executor)).await
                    {
                        instances.push((PluginType::$variant, p));
                    }
                )+

                // Phase 2: run all host compatibility probes concurrently.
                let check_futs: Vec<_> = instances
                    .into_iter()
                    .map(|(pt, p)| async move {
                        if !probe_plugin_host_compatibility(p.as_ref(), pt.clone()).await {
                            return None;
                        }
                        let entries = p.required_sudo_commands();
                        if entries.is_empty() { None } else { Some((pt, entries)) }
                    })
                    .collect();

                join_all(check_futs)
                    .await
                    .into_iter()
                    .flatten()
                    .collect()
            }

            /// Collect extension manifest/action pairs from all macro-registered plugins.
            ///
            /// Calls [`PluginBase::extension_manifests`] (static `where Self: Sized` variant) on
            /// each registered plugin type. Plugins without extensions return empty vecs and
            /// contribute nothing. This is the authoritative source for controller-startup
            /// extension seeding for all non-notification plugins.
            pub fn static_plugin_extension_manifests_and_actions() -> Vec<(
                uptrakit_extension_framework::ExtensionManifest,
                Vec<uptrakit_extension_framework::ActionDef>,
            )> {
                let mut result = Vec::new();
                $(
                    let manifests = <$plugin>::extension_manifests();
                    let actions   = <$plugin>::extension_actions();
                    for manifest in manifests {
                        result.push((manifest, actions.clone()));
                    }
                )+
                result
            }

            /// Flat list of all extension manifests from macro-registered plugins.
            pub fn static_plugin_extension_manifests()
            -> Vec<uptrakit_extension_framework::ExtensionManifest> {
                let mut result = Vec::new();
                $(result.extend(<$plugin>::extension_manifests());)+
                result
            }

            /// Flat list of all extension actions from macro-registered plugins.
            pub fn static_plugin_extension_actions()
            -> Vec<uptrakit_extension_framework::ActionDef> {
                let mut result = Vec::new();
                $(result.extend(<$plugin>::extension_actions());)+
                result
            }

            /// Dispatch an extension action to the appropriate plugin handler.
            ///
            /// Routes based on the extension ID prefix declared via `extension_prefix`
            /// in the [`register_plugins!`] macro. Returns `Err` if no plugin handles
            /// the given extension ID.
            pub fn handle_extension_action<'a>(
                ctx: &'a crate::ExtensionActionContext<'a>,
                extension_id: &'a str,
                action_id: &'a str,
                params: serde_json::Value,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = std::result::Result<serde_json::Value, String>>
                        + Send
                        + 'a,
                >,
            > {
                Box::pin(async move {
                    $($(
                        if extension_id.starts_with($ext_prefix) {
                            return $ext_handler(
                                ctx,
                                extension_id,
                                action_id,
                                params,
                            )
                            .await;
                        }
                    )?)+

                    Err(format!("no plugin handles extension '{extension_id}'"))
                })
            }
        }
    };
}

/// Plugin registry for creating and validating plugins.
///
/// This struct provides a centralized API for:
/// - Creating plugin instances from type, config, and executor
/// - Validating plugin configuration and package identifiers
/// - Masking and restoring secrets in configuration
///
/// All dispatch methods (`create_plugin`, `validate_config`,
/// `mask_config_secrets`, `restore_config_secrets`,
/// `create_plugin_for_discovery`, `discovery_plugins`,
/// `validate_package_identifier`) are generated by the [`register_plugins!`]
/// macro from a single declaration. To add a new plugin, add one line to the
/// macro invocation below and implement `validate_identifier` on its config
/// type (no-op `Ok(())` is acceptable for unconstrained identifiers).
pub struct PluginRegistry {
    /// Notification plugins stored directly, keyed by channel type.
    #[cfg(feature = "notifications")]
    pub(crate) notification_plugins:
        std::collections::HashMap<&'static str, std::sync::Arc<dyn PluginBase>>,

    /// Software item lifecycle plugins (e.g. Dashboard Icons enhancement).
    pub(crate) software_item_lifecycle_plugins: Vec<std::sync::Arc<dyn PluginBase>>,
}

register_plugins! {
    ReleasesGithub                => { config: GitHubConfig,               plugin: GitHubPlugin },
    ReleasesGitlab                => { config: GitLabConfig,               plugin: GitLabPlugin },
    ReleasesForgejo               => { config: ForgejoConfig,              plugin: ForgejoPlugin },
    ReleasesDocker                => {
        config: DockerConfig,
        plugin: DockerPlugin,
        extension_prefix: "docker.",
        extension_handler: uptrakit_plugin_releases_docker::extensions::handle_action
    },
    DiscoveryProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig, plugin: ProxmoxHelperScriptsPlugin },
    PackageManagerHomebrew        => { config: HomebrewConfig,             plugin: HomebrewPlugin },
    PackageManagerApt             => { config: AptConfig,                  plugin: AptPlugin },
    PackageManagerDnf             => { config: DnfConfig,                  plugin: DnfPlugin },
    PackageManagerNpm             => { config: NpmConfig,                  plugin: NpmPlugin },
    PackageManagerMas             => { config: MasConfig,                  plugin: MasPlugin },
    PackageManagerPacman          => { config: PacmanConfig,               plugin: PacmanPlugin },
    PackageManagerPkg             => { config: PkgConfig,                  plugin: PkgPlugin },
    PackageManagerApk             => { config: ApkConfig,                  plugin: ApkPlugin },
    PackageManagerSnap            => { config: SnapConfig,                 plugin: SnapPlugin },
    PackageManagerCargo           => { config: CargoConfig,               plugin: CargoPlugin },
    GenericShell                  => { config: ShellConfig,                plugin: ShellPlugin },
    HookSystemd                   => { config: SystemdHookConfig,          plugin: SystemdHookPlugin },
    HookShell                     => { config: ShellHookConfig,            plugin: ShellHookPlugin },
    InfrastructureProxmox         => {
        config: ProxmoxConfig,
        plugin: ProxmoxPlugin,
        extension_prefix: "proxmox.",
        extension_handler: uptrakit_plugin_infrastructure_proxmox::extensions::handle_action
    },
}

impl PluginRegistry {
    /// Validate plugin configuration from string type.
    ///
    /// This is a convenience method that accepts a string plugin type.
    pub fn validate_config_str(plugin_type: &str, config: &serde_json::Value) -> Result<()> {
        let pt: PluginType = plugin_type.parse().map_err(|_| {
            report!(PluginRegistryError::UnknownPluginType(
                plugin_type.to_string()
            ))
        })?;

        Self::validate_config(pt, config)
    }

    /// Mask secrets in plugin configuration JSON (string type version).
    #[must_use]
    pub fn mask_config_secrets_str(
        plugin_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        let Ok(pt) = plugin_type.parse::<PluginType>() else {
            return config.clone();
        };
        Self::mask_config_secrets(pt, config)
    }

    /// Restore masked secrets from existing configuration (string type version).
    pub fn restore_config_secrets_str(
        plugin_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        let Ok(pt) = plugin_type.parse::<PluginType>() else {
            return;
        };
        Self::restore_config_secrets(pt, incoming, existing)
    }

    /// Returns the capabilities declared by the given plugin type.
    ///
    /// Uses compile-time `CAPABILITIES` constants — no instantiation needed.
    /// Returns an empty vec for unknown types.
    pub fn capabilities_for(
        plugin_type: PluginType,
    ) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
        Self::static_capabilities(plugin_type).to_vec()
    }

    /// String-accepting convenience wrapper around [`capabilities_for`].
    pub fn capabilities_for_str(
        plugin_type: &str,
    ) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
        let Ok(pt) = plugin_type.parse::<PluginType>() else {
            return vec![];
        };
        Self::capabilities_for(pt)
    }

    /// Create a plugin registry without notification support.
    ///
    /// Used by agent crates that don't need notification channel operations.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "notifications")]
            notification_plugins: std::collections::HashMap::new(),
            software_item_lifecycle_plugins: Vec::new(),
        }
    }

    /// Create a plugin registry with notification support.
    ///
    /// Registers compiled-in notification plugins based on feature flags.
    /// The `config` carries deployment-level settings (e.g. whether to
    /// allow private webhook URLs).
    #[cfg(feature = "notifications")]
    pub fn with_notifications(
        // Used conditionally by feature-gated notification channel blocks below.
        #[allow(unused_variables)] config: NotificationRegistryConfig,
    ) -> uptrakit_notification_plugin_core::Result<Self> {
        #[allow(unused_mut)] // mutated conditionally by feature-gated channel insertions
        let mut plugins: std::collections::HashMap<
            &'static str,
            std::sync::Arc<dyn PluginBase>,
        > = std::collections::HashMap::new();

        #[cfg(feature = "notifications-webhook")]
        {
            let catalog_config = uptrakit_plugin_infrastructure_core::CatalogConfig {
                allow_private_urls: config.allow_private_urls,
                ..Default::default()
            };
            let transport_fn = uptrakit_notification_plugin_webhook::DESCRIPTOR
                .roles
                .notification_transport
                .expect("webhook descriptor has notification_transport");
            let transport = transport_fn(&catalog_config).map_err(|e| {
                rootcause::report!(
                    uptrakit_notification_plugin_core::NotificationPluginError::HttpClientBuild(
                        e.to_string()
                    )
                )
            })?;
            plugins.insert(
                "webhook",
                std::sync::Arc::new(NotificationPluginAdapter {
                    descriptor: &uptrakit_notification_plugin_webhook::DESCRIPTOR,
                    transport,
                }),
            );
        }

        #[cfg(feature = "notifications-telegram")]
        {
            let plugin = uptrakit_notification_plugin_telegram::TelegramPlugin::new()?;
            plugins.insert("telegram", std::sync::Arc::new(plugin));
        }

        #[cfg(feature = "notifications-email")]
        {
            plugins.insert(
                "email",
                std::sync::Arc::new(uptrakit_notification_plugin_email::EmailPlugin),
            );
        }

        Ok(Self {
            notification_plugins: plugins,
            software_item_lifecycle_plugins: Vec::new(),
        })
    }

    /// Add a Dashboard Icons enhancement plugin, constructing the cache and spawning the
    /// background refresh loop internally.
    #[cfg(feature = "dashboard-icons")]
    pub fn with_dashboard_icons(
        mut self,
        client: reqwest::Client,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Self {
        let cache = std::sync::Arc::new(
            uptrakit_plugin_enhancement_dashboard_icons::DashboardIconCache::new(client),
        );
        uptrakit_plugin_enhancement_dashboard_icons::DashboardIconCache::spawn_refresh_loop(
            std::sync::Arc::clone(&cache),
            cancel,
        );
        let plugin = uptrakit_plugin_enhancement_dashboard_icons::DashboardIconsPlugin::new(cache);
        self.software_item_lifecycle_plugins
            .push(std::sync::Arc::new(plugin));
        self
    }

    /// Look up a notification plugin by channel type.
    #[cfg(feature = "notifications")]
    pub(crate) fn notification_plugin_ref(
        &self,
        channel_type: &str,
    ) -> Option<&std::sync::Arc<dyn PluginBase>> {
        self.notification_plugins.get(channel_type)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
