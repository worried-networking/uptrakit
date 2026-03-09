use std::sync::Arc;

use rootcause::prelude::*;

use uptrakit_command::CommandExecutor;
use uptrakit_plugin_discovery_proxmox_helper_scripts::{
    ProxmoxHelperScriptsConfig, ProxmoxHelperScriptsPlugin,
};
use uptrakit_plugin_generic_shell::{ShellConfig, ShellPlugin};
use uptrakit_plugin_infrastructure_core::{
    ConfigFormSchema, Plugin, PluginType, SecretMasking, SudoCommandEntry,
};
use uptrakit_plugin_infrastructure_proxmox::{ProxmoxConfig, ProxmoxPlugin};
use uptrakit_plugin_package_manager_apt::{AptConfig, AptPlugin};
use uptrakit_plugin_package_manager_homebrew::{HomebrewConfig, HomebrewPlugin};
use uptrakit_plugin_package_manager_mas::{MasConfig, MasPlugin};
use uptrakit_plugin_package_manager_npm::{NpmConfig, NpmPlugin};
use uptrakit_plugin_releases_docker::{DockerConfig, DockerPlugin};
use uptrakit_plugin_releases_forgejo::{ForgejoConfig, ForgejoPlugin};
use uptrakit_plugin_releases_github::{GitHubConfig, GitHubPlugin};
use uptrakit_plugin_releases_gitlab::{GitLabConfig, GitLabPlugin};

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
            ) -> Result<Box<dyn Plugin>> {
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
            ) -> Result<Box<dyn Plugin>> {
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
            /// [`Plugin::detect_host_compatibility`] check over the provided executor.
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
                // Plugin construction itself has no blocking I/O; the only async
                // call here is `BollardDockerClient::upgrade_to_daemon_client`
                // which returns immediately (it builds the bollard client object
                // without opening a connection).
                let mut instances: Vec<(PluginType, Box<dyn Plugin>)> = Vec::new();
                $(
                    if let Ok(p) = Self::create_plugin_for_discovery(
                        PluginType::$variant, &empty, Arc::clone(&executor)).await
                    {
                        instances.push((PluginType::$variant, p));
                    }
                )+

                // Phase 2: run all host compatibility probes concurrently.
                // Each probe is an independent subprocess or network call; running
                // them in parallel reduces total latency to max(individual) rather
                // than sum(individual).  This is especially important for the Docker
                // probe, which contacts the daemon socket and may take several
                // seconds if Docker Desktop is in a slow or restarting state.
                let check_futs: Vec<_> = instances
                    .into_iter()
                    .map(|(pt, p)| async move {
                        let is_compatible = if p.has_capability(
                            uptrakit_plugin_infrastructure_core::PluginCapability::DetectHostCompatibility)
                        {
                            match p.detect_host_compatibility().await {
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
                        } else {
                            true
                        };

                        if is_compatible {
                            let entries = p.required_sudo_commands();
                            if !entries.is_empty() {
                                return Some((pt, entries));
                            }
                        }
                        None
                    })
                    .collect();

                join_all(check_futs)
                    .await
                    .into_iter()
                    .flatten()
                    .collect()
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
                                ctx.db,
                                ctx.tenant_id,
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
pub struct PluginRegistry;

register_plugins! {
    ReleasesGithub                => { config: GitHubConfig,               plugin: GitHubPlugin },
    ReleasesGitlab                => { config: GitLabConfig,               plugin: GitLabPlugin },
    ReleasesForgejo               => { config: ForgejoConfig,              plugin: ForgejoPlugin },
    ReleasesDocker                => { config: DockerConfig,               plugin: DockerPlugin },
    DiscoveryProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig, plugin: ProxmoxHelperScriptsPlugin },
    PackageManagerHomebrew        => { config: HomebrewConfig,             plugin: HomebrewPlugin },
    PackageManagerApt             => { config: AptConfig,                  plugin: AptPlugin },
    PackageManagerNpm             => { config: NpmConfig,                  plugin: NpmPlugin },
    PackageManagerMas             => { config: MasConfig,                  plugin: MasPlugin },
    GenericShell                  => { config: ShellConfig,                plugin: ShellPlugin },
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_command::LocalCommandExecutor;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    #[test]
    fn parse_known_plugin_types() {
        assert_eq!(
            "releases_github".parse::<PluginType>().ok(),
            Some(PluginType::ReleasesGithub)
        );
        assert_eq!(
            "releases_gitlab".parse::<PluginType>().ok(),
            Some(PluginType::ReleasesGitlab)
        );
        assert_eq!(
            "releases_forgejo".parse::<PluginType>().ok(),
            Some(PluginType::ReleasesForgejo)
        );
        assert_eq!(
            "releases_docker".parse::<PluginType>().ok(),
            Some(PluginType::ReleasesDocker)
        );
        assert_eq!(
            "discovery_proxmox_helper_scripts"
                .parse::<PluginType>()
                .ok(),
            Some(PluginType::DiscoveryProxmoxHelperScripts)
        );
        assert_eq!(
            "package_manager_homebrew".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerHomebrew)
        );
        assert_eq!(
            "package_manager_apt".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerApt)
        );
        assert_eq!(
            "generic_shell".parse::<PluginType>().ok(),
            Some(PluginType::GenericShell)
        );
        assert!("unknown".parse::<PluginType>().is_err());
        // Old wire string is no longer a known type
        assert!("docker_registry".parse::<PluginType>().is_err());
    }

    #[test]
    fn validate_valid_github_config() {
        // Empty config is valid — all fields are optional.
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config(PluginType::ReleasesGithub, &config).is_ok());
    }

    #[test]
    fn validate_valid_github_config_with_token() {
        let config = serde_json::json!({
            "auth_token": "ghp_test",
            "include_prereleases": false,
            "tag_strip_prefix": "v"
        });
        assert!(PluginRegistry::validate_config(PluginType::ReleasesGithub, &config).is_ok());
    }

    #[test]
    fn validate_invalid_github_config_bad_regex() {
        let config = serde_json::json!({
            "asset_patterns": ["[invalid"]
        });
        assert!(PluginRegistry::validate_config(PluginType::ReleasesGithub, &config).is_err());
    }

    #[test]
    fn validate_valid_docker_config() {
        // Empty config is valid for Docker (no required fields)
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config(PluginType::ReleasesDocker, &config).is_ok());
    }

    #[test]
    fn validate_docker_config_old_semver_fields_are_ignored() {
        // Configs stored before the digest-tracking refactor may contain
        // tracking_mode / tag_patterns / page_size — they must be silently ignored.
        let config = serde_json::json!({
            "tracking_mode": "semver_tags",
            "tag_patterns": ["^v[0-9]+"],
            "page_size": 500
        });
        assert!(
            PluginRegistry::validate_config(PluginType::ReleasesDocker, &config).is_ok(),
            "old semver fields should be silently ignored"
        );
    }

    #[test]
    fn validate_proxmox_helper_scripts_config() {
        // PHS config is always `{}`; validation always succeeds.
        let config = serde_json::json!({});
        assert!(
            PluginRegistry::validate_config(PluginType::DiscoveryProxmoxHelperScripts, &config)
                .is_ok()
        );
    }

    #[test]
    fn validate_config_str_valid() {
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config_str("releases_github", &config).is_ok());
    }

    #[test]
    fn validate_config_str_unknown_type() {
        let config = serde_json::json!({});
        let result = PluginRegistry::validate_config_str("unknown", &config);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("unknown plugin type"));
    }

    #[tokio::test]
    async fn create_plugin_github() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::ReleasesGithub, &config, test_executor())
                .await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn create_plugin_docker() {
        // Empty config is valid
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::ReleasesDocker, &config, test_executor())
                .await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn create_plugin_for_discovery_docker() {
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin_for_discovery(
            PluginType::ReleasesDocker,
            &config,
            test_executor(),
        )
        .await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn create_plugin_proxmox() {
        // PHS config is always `{}`; extra fields are ignored during deserialization.
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin(
            PluginType::DiscoveryProxmoxHelperScripts,
            &config,
            test_executor(),
        )
        .await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn proxmox_plugin_capabilities() {
        // PHS declares DiscoverLocalSoftware and DetectHostCompatibility.
        // RefreshPackageIndex must not be present.
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin(
            PluginType::DiscoveryProxmoxHelperScripts,
            &config,
            test_executor(),
        )
        .await
        .expect("create");
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ));
        assert!(!plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
        ));
    }

    #[test]
    fn mask_config_secrets_proxmox_is_noop() {
        // PHS has no secret fields; masking returns an equivalent empty object.
        let config = serde_json::json!({});
        let masked =
            PluginRegistry::mask_config_secrets(PluginType::DiscoveryProxmoxHelperScripts, &config);
        assert_eq!(masked, serde_json::json!({}));
    }

    #[test]
    fn restore_config_secrets_proxmox_is_noop() {
        // PHS has no secret fields; restoring is a no-op.
        let mut incoming = serde_json::json!({});
        let existing = serde_json::json!({});
        PluginRegistry::restore_config_secrets(
            PluginType::DiscoveryProxmoxHelperScripts,
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming, serde_json::json!({}));
    }

    #[tokio::test]
    async fn create_plugin_homebrew() {
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin(
            PluginType::PackageManagerHomebrew,
            &config,
            test_executor(),
        )
        .await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn create_plugin_homebrew_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        let plugin = PluginRegistry::create_plugin(
            PluginType::PackageManagerHomebrew,
            &config,
            test_executor(),
        )
        .await;
        assert!(plugin.is_ok());
    }

    #[test]
    fn validate_homebrew_config() {
        let config = serde_json::json!({});
        assert!(
            PluginRegistry::validate_config(PluginType::PackageManagerHomebrew, &config).is_ok()
        );
    }

    #[test]
    fn validate_homebrew_config_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        assert!(
            PluginRegistry::validate_config(PluginType::PackageManagerHomebrew, &config).is_ok()
        );
    }

    #[test]
    fn validate_homebrew_config_invalid_package_type() {
        let config = serde_json::json!({"package_type": "invalid"});
        assert!(
            PluginRegistry::validate_config(PluginType::PackageManagerHomebrew, &config).is_err()
        );
    }

    #[tokio::test]
    async fn homebrew_plugin_capabilities() {
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin(
            PluginType::PackageManagerHomebrew,
            &config,
            test_executor(),
        )
        .await
        .unwrap();
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ));
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
        ));
    }

    #[tokio::test]
    async fn docker_plugin_capabilities() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::ReleasesDocker, &config, test_executor())
                .await
                .unwrap();
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ));
        assert!(!plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
        ));
    }

    #[test]
    fn discovery_plugins_includes_docker() {
        let types = PluginRegistry::discovery_plugins();
        assert!(
            types.contains(&PluginType::ReleasesDocker),
            "Docker should be in discovery_plugins()"
        );
    }

    #[tokio::test]
    async fn all_required_sudo_commands_includes_apt() {
        let entries = PluginRegistry::all_required_sudo_commands().await;
        let apt_entry = entries
            .iter()
            .find(|(pt, _)| *pt == PluginType::PackageManagerApt)
            .expect("Apt should have sudo command entries");
        assert!(!apt_entry.1.is_empty());
        assert_eq!(apt_entry.1[0].command, "apt-get");
    }

    #[tokio::test]
    async fn all_required_sudo_commands_no_duplicates_per_plugin() {
        let entries = PluginRegistry::all_required_sudo_commands().await;
        // All entries in results should have non-empty command lists
        for (pt, cmds) in &entries {
            assert!(
                !cmds.is_empty(),
                "plugin {pt} has empty sudo command list but was included"
            );
        }
    }

    // ── compatible_sudo_commands_for_host tests ───────────────────────────

    #[tokio::test]
    async fn compatible_sudo_commands_for_host_returns_valid_entries() {
        let executor = test_executor();
        let entries = PluginRegistry::compatible_sudo_commands_for_host(executor).await;
        // Every included plugin must have a non-empty command list.
        for (pt, cmds) in &entries {
            assert!(
                !cmds.is_empty(),
                "plugin {pt} returned an empty sudo command list but was included"
            );
        }
    }

    #[tokio::test]
    async fn compatible_sudo_commands_for_host_excludes_phs_on_non_phs_host() {
        // On any host that lacks /usr/bin/update the PHS plugin must not be
        // included — its helper scripts must not be installed on non-Proxmox
        // machines (e.g. Flatcar Linux with a read-only /usr/local/bin).
        let executor = test_executor();
        let entries = PluginRegistry::compatible_sudo_commands_for_host(executor).await;
        let phs_entry = entries
            .iter()
            .find(|(pt, _)| *pt == PluginType::DiscoveryProxmoxHelperScripts);
        assert!(
            phs_entry.is_none(),
            "PHS must not be included on a non-PHS host (no /usr/bin/update found)"
        );
    }

    #[tokio::test]
    async fn boxed_plugin_preserves_type() {
        let github_config = serde_json::json!({});
        let github = PluginRegistry::create_plugin(
            PluginType::ReleasesGithub,
            &github_config,
            test_executor(),
        )
        .await
        .expect("create github");
        assert_eq!(github.plugin_type(), PluginType::ReleasesGithub);

        let docker_config = serde_json::json!({});
        let docker = PluginRegistry::create_plugin(
            PluginType::ReleasesDocker,
            &docker_config,
            test_executor(),
        )
        .await
        .expect("create docker");
        assert_eq!(docker.plugin_type(), PluginType::ReleasesDocker);

        let proxmox_config = serde_json::json!({});
        let proxmox = PluginRegistry::create_plugin(
            PluginType::DiscoveryProxmoxHelperScripts,
            &proxmox_config,
            test_executor(),
        )
        .await
        .expect("create proxmox");
        assert_eq!(
            proxmox.plugin_type(),
            PluginType::DiscoveryProxmoxHelperScripts
        );

        let homebrew_config = serde_json::json!({});
        let homebrew = PluginRegistry::create_plugin(
            PluginType::PackageManagerHomebrew,
            &homebrew_config,
            test_executor(),
        )
        .await
        .expect("create homebrew");
        assert_eq!(homebrew.plugin_type(), PluginType::PackageManagerHomebrew);

        let apt_config = serde_json::json!({});
        let apt = PluginRegistry::create_plugin(
            PluginType::PackageManagerApt,
            &apt_config,
            test_executor(),
        )
        .await
        .expect("create apt");
        assert_eq!(apt.plugin_type(), PluginType::PackageManagerApt);
    }

    #[test]
    fn mask_config_secrets_homebrew() {
        let config = serde_json::json!({"package_type": "formula"});
        let masked =
            PluginRegistry::mask_config_secrets(PluginType::PackageManagerHomebrew, &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn mask_config_secrets_github() {
        let config = serde_json::json!({
            "auth_token": "ghp_secret"
        });
        let masked = PluginRegistry::mask_config_secrets(PluginType::ReleasesGithub, &config);
        assert_eq!(masked["auth_token"], "***");
    }

    #[test]
    fn mask_config_secrets_github_always_shows_field() {
        // Even with no auth_token in input, masked output always includes the field.
        let config = serde_json::json!({});
        let masked = PluginRegistry::mask_config_secrets(PluginType::ReleasesGithub, &config);
        assert_eq!(masked["auth_token"], "***");
    }

    #[test]
    fn restore_config_secrets_github() {
        let mut incoming = serde_json::json!({
            "auth_token": "***"
        });
        let existing = serde_json::json!({
            "auth_token": "ghp_real_token"
        });
        PluginRegistry::restore_config_secrets(
            PluginType::ReleasesGithub,
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["auth_token"], "ghp_real_token");
    }

    #[tokio::test]
    async fn create_plugin_apt() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor())
                .await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn create_plugin_apt_all_filter() {
        let config = serde_json::json!({"discovery_filter": "all"});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor())
                .await;
        assert!(plugin.is_ok());
    }

    #[test]
    fn validate_apt_config() {
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config(PluginType::PackageManagerApt, &config).is_ok());
    }

    #[test]
    fn validate_apt_config_invalid_filter_fails() {
        let config = serde_json::json!({"discovery_filter": "unknown"});
        assert!(PluginRegistry::validate_config(PluginType::PackageManagerApt, &config).is_err());
    }

    #[tokio::test]
    async fn apt_plugin_capabilities() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor())
                .await
                .unwrap();
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ));
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
        ));
    }

    #[test]
    fn mask_config_secrets_apt() {
        let config = serde_json::json!({"discovery_filter": "manual"});
        let masked = PluginRegistry::mask_config_secrets(PluginType::PackageManagerApt, &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn validate_package_identifier_apt_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::PackageManagerApt, "nginx")
                .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_apt_uppercase_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::PackageManagerApt, "Nginx")
                .is_err()
        );
    }

    // ── Shell plugin tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn create_plugin_shell_version_only() {
        let config = serde_json::json!({"version_command": "myapp --version"});
        let plugin =
            PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor()).await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn create_plugin_shell_update_only() {
        let config = serde_json::json!({"update_command": "apt-get install -y myapp"});
        let plugin =
            PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor()).await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn create_plugin_shell_both() {
        let config = serde_json::json!({
            "version_command": "myapp --version",
            "update_command": "apt-get install -y myapp"
        });
        let plugin =
            PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor()).await;
        assert!(plugin.is_ok());
    }

    #[test]
    fn validate_config_shell_both_none_fails() {
        // Empty config — both commands absent — must fail validation.
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config(PluginType::GenericShell, &config).is_err());
    }

    // ── validate_package_identifier GitHub tests ──────────────────────────

    #[test]
    fn validate_package_identifier_github_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesGithub,
                "octocat/hello-world"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_github_no_slash_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesGithub, "octocat")
                .is_err()
        );
    }

    #[test]
    fn validate_package_identifier_github_traversal_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesGithub,
                "octocat/../evil"
            )
            .is_err()
        );
    }

    #[test]
    fn validate_package_identifier_github_empty_repo_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesGithub, "octocat/")
                .is_err()
        );
    }

    // ── validate_package_identifier GitLab tests ──────────────────────────

    #[test]
    fn validate_package_identifier_gitlab_simple_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesGitlab,
                "owner/project"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_gitlab_nested_namespace_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesGitlab,
                "group/subgroup/project"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_gitlab_no_slash_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesGitlab, "project")
                .is_err()
        );
    }

    #[test]
    fn validate_package_identifier_gitlab_traversal_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesGitlab,
                "../evil/project"
            )
            .is_err()
        );
    }

    #[test]
    fn validate_package_identifier_gitlab_empty_component_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesGitlab,
                "owner//project"
            )
            .is_err()
        );
    }

    // ── validate_package_identifier Forgejo tests ─────────────────────────

    #[test]
    fn validate_package_identifier_forgejo_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesForgejo, "owner/repo")
                .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_forgejo_no_slash_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesForgejo, "owner")
                .is_err()
        );
    }

    #[test]
    fn validate_package_identifier_forgejo_traversal_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesForgejo,
                "owner/../evil"
            )
            .is_err()
        );
    }

    #[test]
    fn validate_package_identifier_forgejo_empty_repo_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesForgejo, "owner/")
                .is_err()
        );
    }

    // ── PluginType::Other(String) behaviour ──────────────────────────────

    /// `Other(String)` received from a newer server must fail gracefully at
    /// the registry level (unknown type) rather than causing a deserialization
    /// panic or silent data loss.
    #[tokio::test]
    async fn create_plugin_other_returns_unknown_type_error() {
        let config = serde_json::json!({});
        let Err(err) = PluginRegistry::create_plugin(
            PluginType::Other("winget".to_string()),
            &config,
            test_executor(),
        )
        .await
        else {
            panic!("expected Err for Other plugin type");
        };
        assert!(err.to_string().contains("unknown plugin type"));
    }

    #[test]
    fn validate_config_other_returns_unknown_type_error() {
        let config = serde_json::json!({});
        let result =
            PluginRegistry::validate_config(PluginType::Other("winget".to_string()), &config);
        assert!(result.is_err());
    }

    /// `mask_config_secrets` for an `Other` plugin type returns the config
    /// unchanged (no masking possible for an unknown plugin).
    #[test]
    fn mask_config_secrets_other_returns_config_unchanged() {
        let config = serde_json::json!({"token": "secret", "repo": "something"});
        let result =
            PluginRegistry::mask_config_secrets(PluginType::Other("winget".to_string()), &config);
        assert_eq!(result, config);
    }

    // ── validate_package_identifier ───────────────────────────────────────

    /// `Other` always returns `Ok(())`.
    #[test]
    fn validate_package_identifier_other_is_permissive() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::Other("flatpak".to_string()),
                "org.example.App"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_docker_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "nginx")
                .is_ok()
        );
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesDocker,
                "ghcr.io/owner/app:latest"
            )
            .is_ok()
        );
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::ReleasesDocker,
                "myuser/app:v2"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_docker_invalid() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "").is_err()
        );
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "nginx latest")
                .is_err()
        );
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::ReleasesDocker, "ghcr.io//app")
                .is_err()
        );
    }

    // ── capabilities_for / capabilities_for_str ───────────────────────────

    #[test]
    fn capabilities_for_docker_includes_discover() {
        let caps = PluginRegistry::capabilities_for(PluginType::ReleasesDocker);
        assert!(
            caps.contains(
                &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
            ),
            "Docker plugin should declare DiscoverLocalSoftware"
        );
    }

    #[test]
    fn capabilities_for_github_is_empty() {
        let caps = PluginRegistry::capabilities_for(PluginType::ReleasesGithub);
        assert!(
            !caps.contains(
                &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
            ),
            "GitHub releases plugin must not declare DiscoverLocalSoftware"
        );
    }

    #[test]
    fn capabilities_for_str_docker_includes_discover() {
        let caps = PluginRegistry::capabilities_for_str("releases_docker");
        assert!(
            caps.contains(
                &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
            ),
            "releases_docker should declare DiscoverLocalSoftware via string lookup"
        );
    }

    #[test]
    fn capabilities_for_str_github_has_no_discover() {
        let caps = PluginRegistry::capabilities_for_str("releases_github");
        assert!(
            !caps.contains(
                &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
            ),
            "releases_github must not declare DiscoverLocalSoftware"
        );
    }

    #[test]
    fn capabilities_for_str_unknown_returns_empty() {
        let caps = PluginRegistry::capabilities_for_str("unknown_type");
        assert!(
            caps.is_empty(),
            "Unknown plugin type should return an empty capabilities vec"
        );
    }

    #[test]
    fn capabilities_for_str_generic_shell_has_no_discover() {
        let caps = PluginRegistry::capabilities_for_str("generic_shell");
        assert!(
            !caps.contains(
                &uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
            ),
            "generic_shell must not declare DiscoverLocalSoftware"
        );
    }

    // ── npm plugin tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_plugin_npm() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerNpm, &config, test_executor())
                .await;
        assert!(plugin.is_ok());
    }

    #[tokio::test]
    async fn create_plugin_npm_with_prereleases() {
        let config = serde_json::json!({"include_prereleases": true});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerNpm, &config, test_executor())
                .await;
        assert!(plugin.is_ok());
    }

    #[test]
    fn validate_npm_config() {
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config(PluginType::PackageManagerNpm, &config).is_ok());
    }

    #[tokio::test]
    async fn npm_plugin_capabilities() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerNpm, &config, test_executor())
                .await
                .unwrap();
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ));
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::ControllerSideFetchReleases
        ));
        assert!(!plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
        ));
    }

    #[test]
    fn mask_config_secrets_npm_is_noop() {
        let config = serde_json::json!({"include_prereleases": false});
        let masked = PluginRegistry::mask_config_secrets(PluginType::PackageManagerNpm, &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn validate_package_identifier_npm_plain_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::PackageManagerNpm, "n8n")
                .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_npm_scoped_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(
                PluginType::PackageManagerNpm,
                "@angular/cli"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_npm_uppercase_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::PackageManagerNpm, "MyPackage")
                .is_err()
        );
    }

    #[tokio::test]
    async fn all_required_sudo_commands_includes_npm() {
        let entries = PluginRegistry::all_required_sudo_commands().await;
        let npm_entry = entries
            .iter()
            .find(|(pt, _)| *pt == PluginType::PackageManagerNpm)
            .expect("npm should have sudo command entries");
        assert!(!npm_entry.1.is_empty());
        assert_eq!(npm_entry.1[0].command, "npm");
    }

    #[test]
    fn discovery_plugins_includes_npm() {
        let types = PluginRegistry::discovery_plugins();
        assert!(
            types.contains(&PluginType::PackageManagerNpm),
            "npm should be in discovery_plugins()"
        );
    }

    #[test]
    fn discovery_plugins_includes_mas() {
        let types = PluginRegistry::discovery_plugins();
        assert!(
            types.contains(&PluginType::PackageManagerMas),
            "Mac App Store plugin should be in discovery_plugins()"
        );
    }
}
