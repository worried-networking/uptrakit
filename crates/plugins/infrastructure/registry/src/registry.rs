use std::sync::Arc;

use rootcause::prelude::*;

use uptrakit_command::CommandExecutor;
use uptrakit_plugin_discovery_proxmox_helper_scripts::{
    ProxmoxHelperScriptsConfig, ProxmoxHelperScriptsPlugin,
};
use uptrakit_plugin_generic_shell::{ShellConfig, ShellPlugin};
use uptrakit_plugin_infrastructure_core::{Plugin, PluginType, SecretMasking, SudoCommandEntry};
use uptrakit_plugin_package_manager_apt::{AptConfig, AptPlugin};
use uptrakit_plugin_package_manager_homebrew::{HomebrewConfig, HomebrewPlugin};
use uptrakit_plugin_releases_docker::{DockerConfig, DockerPlugin};
use uptrakit_plugin_releases_github::{GitHubConfig, GitHubPlugin};

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

/// Generates the six `PluginRegistry` dispatch methods from a single
/// declaration list, eliminating manually-maintained match arms.
macro_rules! register_plugins {
    ($(
        $variant:ident => { config: $config:ty, plugin: $plugin:ty }
    ),+ $(,)?) => {
        impl PluginRegistry {
            /// Create a plugin instance from plugin type, config, and executor.
            ///
            /// Deserializes the config, validates it, and constructs the plugin.
            /// All plugins follow the same pattern: deserialize → validate →
            /// construct.
            pub fn create_plugin(
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
                            let plugin = <$plugin>::new(typed_config, executor)
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
            pub fn create_plugin_for_discovery(
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
                            let plugin = <$plugin>::new(typed_config, executor)
                                .map_err(|e| report!(PluginRegistryError::Instantiation(e.to_string())))?;
                            Ok(Box::new(plugin))
                        }
                    )+
                    _ => Err(report!(PluginRegistryError::UnknownPluginType(format!(
                        "{plugin_type}"
                    )))),
                }
            }

            /// Returns all plugin types that have the `DiscoverLocalSoftware` capability.
            ///
            /// Auto-derived from the macro registration — no manual list needed.
            pub fn discovery_plugins() -> Vec<PluginType> {
                let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
                let empty = serde_json::Value::Object(serde_json::Map::new());
                let mut result = Vec::new();
                $(
                    if let Ok(p) = Self::create_plugin_for_discovery(
                        PluginType::$variant, &empty, executor.clone())
                    {
                        if p.has_capability(uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware) {
                            result.push(PluginType::$variant);
                        }
                    }
                )+
                result
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
            pub fn all_required_sudo_commands() -> Vec<(PluginType, Vec<SudoCommandEntry>)> {
                let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
                let empty = serde_json::Value::Object(serde_json::Map::new());
                let mut result = Vec::new();
                $(
                    if let Ok(p) = Self::create_plugin_for_discovery(
                        PluginType::$variant, &empty, executor.clone())
                    {
                        let entries = p.required_sudo_commands();
                        if !entries.is_empty() {
                            result.push((PluginType::$variant, entries));
                        }
                    }
                )+
                result
            }
        }
    };
}

/// Plugin registry for creating and validating plugins.
///
/// This struct provides a centralized API for:
/// - Creating plugin instances from type, config, and executor
/// - Validating plugin configuration
/// - Masking and restoring secrets in configuration
///
/// All six dispatch methods (`create_plugin`, `validate_config`,
/// `mask_config_secrets`, `restore_config_secrets`,
/// `create_plugin_for_discovery`, `discovery_plugins`) are
/// generated by the [`register_plugins!`] macro from a single declaration.
/// To add a new plugin, add one line to the macro invocation below.
pub struct PluginRegistry;

register_plugins! {
    ReleasesGithub                => { config: GitHubConfig,                   plugin: GitHubPlugin },
    ReleasesDocker                => { config: DockerConfig,                   plugin: DockerPlugin },
    DiscoveryProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig,     plugin: ProxmoxHelperScriptsPlugin },
    PackageManagerHomebrew        => { config: HomebrewConfig,                 plugin: HomebrewPlugin },
    PackageManagerApt             => { config: AptConfig,                      plugin: AptPlugin },
    GenericShell                  => { config: ShellConfig,                    plugin: ShellPlugin },
}

impl PluginRegistry {
    /// Validate a package identifier for the given plugin type.
    ///
    /// Returns `Ok(())` for plugin types that have no identifier constraints.
    /// Returns `Err(message)` when the identifier violates plugin-specific rules.
    pub fn validate_package_identifier(
        plugin_type: PluginType,
        value: &str,
    ) -> std::result::Result<(), String> {
        match plugin_type {
            PluginType::ReleasesGithub => {
                uptrakit_plugin_releases_github::validate_identifier(value)
            }
            PluginType::ReleasesDocker => {
                uptrakit_plugin_releases_docker::validate_identifier(value)
            }
            PluginType::PackageManagerHomebrew => {
                uptrakit_plugin_package_manager_homebrew::validate_identifier(value)
            }
            PluginType::PackageManagerApt => {
                uptrakit_plugin_package_manager_apt::validate_identifier(value)
            }
            _ => Ok(()),
        }
    }

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
    fn validate_invalid_docker_config_zero_page_size() {
        let config = serde_json::json!({ "page_size": 0 });
        assert!(PluginRegistry::validate_config(PluginType::ReleasesDocker, &config).is_err());
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

    #[test]
    fn create_plugin_github() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::ReleasesGithub, &config, test_executor());
        assert!(plugin.is_ok());
    }

    #[test]
    fn create_plugin_docker() {
        // Empty config is valid
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::ReleasesDocker, &config, test_executor());
        assert!(plugin.is_ok());
    }

    #[test]
    fn create_plugin_for_discovery_docker() {
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin_for_discovery(
            PluginType::ReleasesDocker,
            &config,
            test_executor(),
        );
        assert!(plugin.is_ok());
    }

    #[test]
    fn create_plugin_proxmox() {
        // PHS config is always `{}`; extra fields are ignored during deserialization.
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin(
            PluginType::DiscoveryProxmoxHelperScripts,
            &config,
            test_executor(),
        );
        assert!(plugin.is_ok());
    }

    #[test]
    fn proxmox_plugin_capabilities() {
        // PHS is discovery-only; RefreshPackageIndex capability must not be present.
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin(
            PluginType::DiscoveryProxmoxHelperScripts,
            &config,
            test_executor(),
        )
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

    #[test]
    fn create_plugin_homebrew() {
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin(
            PluginType::PackageManagerHomebrew,
            &config,
            test_executor(),
        );
        assert!(plugin.is_ok());
    }

    #[test]
    fn create_plugin_homebrew_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        let plugin = PluginRegistry::create_plugin(
            PluginType::PackageManagerHomebrew,
            &config,
            test_executor(),
        );
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

    #[test]
    fn homebrew_plugin_capabilities() {
        let config = serde_json::json!({});
        let plugin = PluginRegistry::create_plugin(
            PluginType::PackageManagerHomebrew,
            &config,
            test_executor(),
        )
        .unwrap();
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::DiscoverLocalSoftware
        ));
        assert!(plugin.has_capability(
            uptrakit_plugin_infrastructure_core::PluginCapability::RefreshPackageIndex
        ));
    }

    #[test]
    fn docker_plugin_capabilities() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::ReleasesDocker, &config, test_executor())
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

    #[test]
    fn all_required_sudo_commands_includes_apt() {
        let entries = PluginRegistry::all_required_sudo_commands();
        let apt_entry = entries
            .iter()
            .find(|(pt, _)| *pt == PluginType::PackageManagerApt)
            .expect("Apt should have sudo command entries");
        assert!(!apt_entry.1.is_empty());
        assert_eq!(apt_entry.1[0].command, "apt-get");
    }

    #[test]
    fn all_required_sudo_commands_no_duplicates_per_plugin() {
        let entries = PluginRegistry::all_required_sudo_commands();
        // All entries in results should have non-empty command lists
        for (pt, cmds) in &entries {
            assert!(
                !cmds.is_empty(),
                "plugin {pt} has empty sudo command list but was included"
            );
        }
    }

    #[test]
    fn boxed_plugin_preserves_type() {
        let github_config = serde_json::json!({});
        let github = PluginRegistry::create_plugin(
            PluginType::ReleasesGithub,
            &github_config,
            test_executor(),
        )
        .expect("create github");
        assert_eq!(github.plugin_type(), PluginType::ReleasesGithub);

        let docker_config = serde_json::json!({});
        let docker = PluginRegistry::create_plugin(
            PluginType::ReleasesDocker,
            &docker_config,
            test_executor(),
        )
        .expect("create docker");
        assert_eq!(docker.plugin_type(), PluginType::ReleasesDocker);

        let proxmox_config = serde_json::json!({});
        let proxmox = PluginRegistry::create_plugin(
            PluginType::DiscoveryProxmoxHelperScripts,
            &proxmox_config,
            test_executor(),
        )
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
        .expect("create homebrew");
        assert_eq!(homebrew.plugin_type(), PluginType::PackageManagerHomebrew);

        let apt_config = serde_json::json!({});
        let apt = PluginRegistry::create_plugin(
            PluginType::PackageManagerApt,
            &apt_config,
            test_executor(),
        )
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

    #[test]
    fn create_plugin_apt() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor());
        assert!(plugin.is_ok());
    }

    #[test]
    fn create_plugin_apt_all_filter() {
        let config = serde_json::json!({"discovery_filter": "all"});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor());
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

    #[test]
    fn apt_plugin_capabilities() {
        let config = serde_json::json!({});
        let plugin =
            PluginRegistry::create_plugin(PluginType::PackageManagerApt, &config, test_executor())
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

    #[test]
    fn create_plugin_shell_version_only() {
        let config = serde_json::json!({"version_command": "myapp --version"});
        let plugin =
            PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor());
        assert!(plugin.is_ok());
    }

    #[test]
    fn create_plugin_shell_update_only() {
        let config = serde_json::json!({"update_command": "apt-get install -y myapp"});
        let plugin =
            PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor());
        assert!(plugin.is_ok());
    }

    #[test]
    fn create_plugin_shell_both() {
        let config = serde_json::json!({
            "version_command": "myapp --version",
            "update_command": "apt-get install -y myapp"
        });
        let plugin =
            PluginRegistry::create_plugin(PluginType::GenericShell, &config, test_executor());
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

    // ── PluginType::Other(String) behaviour ──────────────────────────────

    /// `Other(String)` received from a newer server must fail gracefully at
    /// the registry level (unknown type) rather than causing a deserialization
    /// panic or silent data loss.
    #[test]
    fn create_plugin_other_returns_unknown_type_error() {
        let config = serde_json::json!({});
        let Err(err) = PluginRegistry::create_plugin(
            PluginType::Other("winget".to_string()),
            &config,
            test_executor(),
        ) else {
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
}
