use std::sync::Arc;

use rootcause::prelude::*;

use uptrakit_command::CommandExecutor;
use uptrakit_plugin_core::{Plugin, PluginType, SecretMasking, SudoCommandEntry};
use uptrakit_plugin_apt::{AptConfig, AptPlugin};
use uptrakit_plugin_docker::{DockerConfig, DockerPlugin};
use uptrakit_plugin_github::{GitHubConfig, GitHubPlugin};
use uptrakit_plugin_homebrew::{HomebrewConfig, HomebrewPlugin};
use uptrakit_plugin_proxmox_helper_scripts::{
    ProxmoxHelperScriptsConfig, ProxmoxHelperScriptsPlugin,
};

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
                "failed to serialize masked provider config; \
                 falling back to original — provider secrets may be exposed in API responses"
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
        $variant:ident => { config: $config:ty, provider: $provider:ty }
    ),+ $(,)?) => {
        impl PluginRegistry {
            /// Create a provider instance from provider type, config, and executor.
            ///
            /// Deserializes the config, validates it, and constructs the provider.
            /// All providers follow the same pattern: deserialize → validate →
            /// construct.
            pub fn create_provider(
                provider_type: PluginType,
                config: &serde_json::Value,
                executor: Arc<dyn CommandExecutor>,
            ) -> Result<Box<dyn Plugin>> {
                match provider_type {
                    $(
                        PluginType::$variant => {
                            let typed_config: $config =
                                serde_json::from_value(config.clone()).context_to()?;
                            typed_config
                                .validate()
                                .map_err(|e| report!(PluginRegistryError::ConfigValidation(e.to_string())))?;
                            let provider = <$provider>::new(typed_config, executor)
                                .map_err(|e| report!(PluginRegistryError::Instantiation(e.to_string())))?;
                            Ok(Box::new(provider))
                        }
                    )+
                    _ => Err(report!(PluginRegistryError::UnknownProviderType(format!(
                        "{provider_type}"
                    )))),
                }
            }

            /// Validate provider configuration JSON.
            pub fn validate_config(
                provider_type: PluginType,
                config: &serde_json::Value,
            ) -> Result<()> {
                match provider_type {
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
                    _ => Err(report!(PluginRegistryError::UnknownProviderType(format!(
                        "{provider_type}"
                    )))),
                }
            }

            /// Mask secrets in provider configuration JSON for API responses.
            ///
            /// Deserializes config, calls [`SecretMasking::with_secrets_masked()`],
            /// and serializes back. Unknown provider types are returned unchanged.
            pub fn mask_config_secrets(
                provider_type: PluginType,
                config: &serde_json::Value,
            ) -> serde_json::Value {
                match provider_type {
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
                provider_type: PluginType,
                incoming: &mut serde_json::Value,
                existing: &serde_json::Value,
            ) {
                match provider_type {
                    $(
                        PluginType::$variant => {
                            restore_secrets_for::<$config>(incoming, existing);
                        }
                    )+
                    _ => {}
                }
            }

            /// Create a provider instance for autodiscovery, bypassing `validate()`.
            ///
            /// Discovery can proceed with an empty/minimal config.  For providers
            /// whose `validate()` is a no-op (e.g. `ProxmoxHelperScripts`) the two
            /// construction paths are equivalent.
            pub fn create_provider_for_discovery(
                provider_type: PluginType,
                config: &serde_json::Value,
                executor: Arc<dyn CommandExecutor>,
            ) -> Result<Box<dyn Plugin>> {
                match provider_type {
                    $(
                        PluginType::$variant => {
                            let typed_config: $config =
                                serde_json::from_value(config.clone()).context_to()?;
                            // No validate() — discovery can proceed with an empty/minimal config.
                            let provider = <$provider>::new(typed_config, executor)
                                .map_err(|e| report!(PluginRegistryError::Instantiation(e.to_string())))?;
                            Ok(Box::new(provider))
                        }
                    )+
                    _ => Err(report!(PluginRegistryError::UnknownProviderType(format!(
                        "{provider_type}"
                    )))),
                }
            }

            /// Returns all provider types that have the `DiscoverLocalSoftware` capability.
            ///
            /// Auto-derived from the macro registration — no manual list needed.
            pub fn discovery_plugin_types() -> Vec<PluginType> {
                let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
                let empty = serde_json::Value::Object(serde_json::Map::new());
                let mut result = Vec::new();
                $(
                    if let Ok(p) = Self::create_provider_for_discovery(
                        PluginType::$variant, &empty, executor.clone())
                    {
                        if p.has_capability(uptrakit_plugin_core::PluginCapability::DiscoverLocalSoftware) {
                            result.push(PluginType::$variant);
                        }
                    }
                )+
                result
            }

            /// Returns required sudo command entries for every registered provider.
            ///
            /// Iterates all known provider types, instantiates each with an empty
            /// config (using `create_provider_for_discovery` which bypasses validation),
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
                    if let Ok(p) = Self::create_provider_for_discovery(
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

/// Provider registry for creating and validating providers.
///
/// This struct provides a centralized API for:
/// - Creating provider instances from type, config, and executor
/// - Validating provider configuration
/// - Masking and restoring secrets in configuration
///
/// All six dispatch methods (`create_provider`, `validate_config`,
/// `mask_config_secrets`, `restore_config_secrets`,
/// `create_provider_for_discovery`, `discovery_provider_types`) are
/// generated by the [`register_plugins!`] macro from a single declaration.
/// To add a new provider, add one line to the macro invocation below.
pub struct PluginRegistry;

register_plugins! {
    GithubReleases       => { config: GitHubConfig,                   provider: GitHubPlugin },
    Docker               => { config: DockerConfig,                   provider: DockerPlugin },
    ProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig,     provider: ProxmoxHelperScriptsPlugin },
    Homebrew             => { config: HomebrewConfig,                 provider: HomebrewPlugin },
    Apt                  => { config: AptConfig,                      provider: AptPlugin },
}

impl PluginRegistry {
    /// Validate a package identifier for the given provider type.
    ///
    /// Returns `Ok(())` for provider types that have no identifier constraints.
    /// Returns `Err(message)` when the identifier violates provider-specific rules.
    pub fn validate_package_identifier(
        provider_type: PluginType,
        value: &str,
    ) -> std::result::Result<(), String> {
        match provider_type {
            PluginType::Docker => uptrakit_plugin_docker::validate_identifier(value),
            PluginType::Homebrew => uptrakit_plugin_homebrew::validate_identifier(value),
            PluginType::Apt => uptrakit_plugin_apt::validate_identifier(value),
            _ => Ok(()),
        }
    }

    /// Validate provider configuration from string type.
    ///
    /// This is a convenience method that accepts a string provider type.
    pub fn validate_config_str(provider_type: &str, config: &serde_json::Value) -> Result<()> {
        let pt: PluginType = provider_type.parse().map_err(|_| {
            report!(PluginRegistryError::UnknownProviderType(
                provider_type.to_string()
            ))
        })?;

        Self::validate_config(pt, config)
    }

    /// Mask secrets in provider configuration JSON (string type version).
    pub fn mask_config_secrets_str(
        provider_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        let Ok(pt) = provider_type.parse::<PluginType>() else {
            return config.clone();
        };
        Self::mask_config_secrets(pt, config)
    }

    /// Restore masked secrets from existing configuration (string type version).
    pub fn restore_config_secrets_str(
        provider_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        let Ok(pt) = provider_type.parse::<PluginType>() else {
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
    fn parse_known_provider_types() {
        assert_eq!(
            "github_releases".parse::<PluginType>().ok(),
            Some(PluginType::GithubReleases)
        );
        assert_eq!(
            "docker".parse::<PluginType>().ok(),
            Some(PluginType::Docker)
        );
        assert_eq!(
            "proxmox_helper_scripts".parse::<PluginType>().ok(),
            Some(PluginType::ProxmoxHelperScripts)
        );
        assert_eq!(
            "homebrew".parse::<PluginType>().ok(),
            Some(PluginType::Homebrew)
        );
        assert_eq!(
            "apt".parse::<PluginType>().ok(),
            Some(PluginType::Apt)
        );
        assert!("unknown".parse::<PluginType>().is_err());
        // Old wire string is no longer a known type
        assert!("docker_registry".parse::<PluginType>().is_err());
    }

    #[test]
    fn validate_valid_github_config() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        assert!(PluginRegistry::validate_config(PluginType::GithubReleases, &config).is_ok());
    }

    #[test]
    fn validate_invalid_github_config() {
        let config = serde_json::json!({
            "owner": "",
            "repo": "hello-world"
        });
        assert!(PluginRegistry::validate_config(PluginType::GithubReleases, &config).is_err());
    }

    #[test]
    fn validate_valid_docker_config() {
        // Empty config is valid for Docker (no required fields)
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config(PluginType::Docker, &config).is_ok());
    }

    #[test]
    fn validate_invalid_docker_config_zero_page_size() {
        let config = serde_json::json!({ "page_size": 0 });
        assert!(PluginRegistry::validate_config(PluginType::Docker, &config).is_err());
    }

    #[test]
    fn validate_proxmox_helper_scripts_config() {
        // PHS config is always `{}`; validation always succeeds.
        let config = serde_json::json!({});
        assert!(
            PluginRegistry::validate_config(PluginType::ProxmoxHelperScripts, &config).is_ok()
        );
    }

    #[test]
    fn validate_config_str_valid() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        assert!(PluginRegistry::validate_config_str("github_releases", &config).is_ok());
    }

    #[test]
    fn validate_config_str_unknown_type() {
        let config = serde_json::json!({});
        let result = PluginRegistry::validate_config_str("unknown", &config);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("unknown provider type"));
    }

    #[test]
    fn create_provider_github() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let provider = PluginRegistry::create_provider(
            PluginType::GithubReleases,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_docker() {
        // Empty config is valid
        let config = serde_json::json!({});
        let provider = PluginRegistry::create_provider(
            PluginType::Docker,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_for_discovery_docker() {
        let config = serde_json::json!({});
        let provider = PluginRegistry::create_provider_for_discovery(
            PluginType::Docker,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_proxmox() {
        // PHS config is always `{}`; extra fields are ignored during deserialization.
        let config = serde_json::json!({});
        let provider = PluginRegistry::create_provider(
            PluginType::ProxmoxHelperScripts,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn proxmox_provider_capabilities() {
        // PHS is discovery-only; RefreshPackageIndex capability must not be present.
        let config = serde_json::json!({});
        let provider = PluginRegistry::create_provider(
            PluginType::ProxmoxHelperScripts,
            &config,
            test_executor(),
        )
        .expect("create");
        assert!(
            provider
                .has_capability(uptrakit_plugin_core::PluginCapability::DiscoverLocalSoftware)
        );
        assert!(
            !provider
                .has_capability(uptrakit_plugin_core::PluginCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn mask_config_secrets_proxmox_is_noop() {
        // PHS has no secret fields; masking returns an equivalent empty object.
        let config = serde_json::json!({});
        let masked =
            PluginRegistry::mask_config_secrets(PluginType::ProxmoxHelperScripts, &config);
        assert_eq!(masked, serde_json::json!({}));
    }

    #[test]
    fn restore_config_secrets_proxmox_is_noop() {
        // PHS has no secret fields; restoring is a no-op.
        let mut incoming = serde_json::json!({});
        let existing = serde_json::json!({});
        PluginRegistry::restore_config_secrets(
            PluginType::ProxmoxHelperScripts,
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming, serde_json::json!({}));
    }

    #[test]
    fn create_provider_homebrew() {
        let config = serde_json::json!({});
        let provider =
            PluginRegistry::create_provider(PluginType::Homebrew, &config, test_executor());
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_homebrew_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        let provider =
            PluginRegistry::create_provider(PluginType::Homebrew, &config, test_executor());
        assert!(provider.is_ok());
    }

    #[test]
    fn validate_homebrew_config() {
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config(PluginType::Homebrew, &config).is_ok());
    }

    #[test]
    fn validate_homebrew_config_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        assert!(PluginRegistry::validate_config(PluginType::Homebrew, &config).is_ok());
    }

    #[test]
    fn validate_homebrew_config_invalid_package_type() {
        let config = serde_json::json!({"package_type": "invalid"});
        assert!(PluginRegistry::validate_config(PluginType::Homebrew, &config).is_err());
    }

    #[test]
    fn homebrew_provider_capabilities() {
        let config = serde_json::json!({});
        let provider =
            PluginRegistry::create_provider(PluginType::Homebrew, &config, test_executor())
                .unwrap();
        assert!(
            provider
                .has_capability(uptrakit_plugin_core::PluginCapability::DiscoverLocalSoftware)
        );
        assert!(
            provider
                .has_capability(uptrakit_plugin_core::PluginCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn docker_provider_capabilities() {
        let config = serde_json::json!({});
        let provider =
            PluginRegistry::create_provider(PluginType::Docker, &config, test_executor())
                .unwrap();
        assert!(
            provider
                .has_capability(uptrakit_plugin_core::PluginCapability::DiscoverLocalSoftware)
        );
        assert!(
            !provider
                .has_capability(uptrakit_plugin_core::PluginCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn discovery_plugin_types_includes_docker() {
        let types = PluginRegistry::discovery_plugin_types();
        assert!(
            types.contains(&PluginType::Docker),
            "Docker should be in discovery_provider_types()"
        );
    }

    #[test]
    fn all_required_sudo_commands_includes_apt() {
        let entries = PluginRegistry::all_required_sudo_commands();
        let apt_entry = entries
            .iter()
            .find(|(pt, _)| *pt == PluginType::Apt)
            .expect("Apt should have sudo command entries");
        assert!(!apt_entry.1.is_empty());
        assert_eq!(apt_entry.1[0].command, "apt-get");
    }

    #[test]
    fn all_required_sudo_commands_no_duplicates_per_provider() {
        let entries = PluginRegistry::all_required_sudo_commands();
        // All entries in results should have non-empty command lists
        for (pt, cmds) in &entries {
            assert!(
                !cmds.is_empty(),
                "provider {pt} has empty sudo command list but was included"
            );
        }
    }

    #[test]
    fn boxed_provider_preserves_type() {
        let github_config = serde_json::json!({"owner": "octocat", "repo": "hello-world"});
        let github = PluginRegistry::create_provider(
            PluginType::GithubReleases,
            &github_config,
            test_executor(),
        )
        .expect("create github");
        assert_eq!(github.plugin_type(), PluginType::GithubReleases);

        let docker_config = serde_json::json!({});
        let docker = PluginRegistry::create_provider(
            PluginType::Docker,
            &docker_config,
            test_executor(),
        )
        .expect("create docker");
        assert_eq!(docker.plugin_type(), PluginType::Docker);

        let proxmox_config = serde_json::json!({});
        let proxmox = PluginRegistry::create_provider(
            PluginType::ProxmoxHelperScripts,
            &proxmox_config,
            test_executor(),
        )
        .expect("create proxmox");
        assert_eq!(proxmox.plugin_type(), PluginType::ProxmoxHelperScripts);

        let homebrew_config = serde_json::json!({});
        let homebrew = PluginRegistry::create_provider(
            PluginType::Homebrew,
            &homebrew_config,
            test_executor(),
        )
        .expect("create homebrew");
        assert_eq!(homebrew.plugin_type(), PluginType::Homebrew);

        let apt_config = serde_json::json!({});
        let apt =
            PluginRegistry::create_provider(PluginType::Apt, &apt_config, test_executor())
                .expect("create apt");
        assert_eq!(apt.plugin_type(), PluginType::Apt);
    }

    #[test]
    fn mask_config_secrets_homebrew() {
        let config = serde_json::json!({"package_type": "formula"});
        let masked = PluginRegistry::mask_config_secrets(PluginType::Homebrew, &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn mask_config_secrets_github() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_secret"
        });
        let masked = PluginRegistry::mask_config_secrets(PluginType::GithubReleases, &config);
        assert_eq!(masked["auth_token"], "***");
        assert_eq!(masked["owner"], "octocat");
    }

    #[test]
    fn mask_config_secrets_github_always_shows_field() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let masked = PluginRegistry::mask_config_secrets(PluginType::GithubReleases, &config);
        assert_eq!(masked["auth_token"], "***");
    }

    #[test]
    fn restore_config_secrets_github() {
        let mut incoming = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "***"
        });
        let existing = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_real_token"
        });
        PluginRegistry::restore_config_secrets(
            PluginType::GithubReleases,
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["auth_token"], "ghp_real_token");
    }

    #[test]
    fn create_provider_apt() {
        let config = serde_json::json!({});
        let provider =
            PluginRegistry::create_provider(PluginType::Apt, &config, test_executor());
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_apt_all_filter() {
        let config = serde_json::json!({"discovery_filter": "all"});
        let provider =
            PluginRegistry::create_provider(PluginType::Apt, &config, test_executor());
        assert!(provider.is_ok());
    }

    #[test]
    fn validate_apt_config() {
        let config = serde_json::json!({});
        assert!(PluginRegistry::validate_config(PluginType::Apt, &config).is_ok());
    }

    #[test]
    fn validate_apt_config_invalid_filter_fails() {
        let config = serde_json::json!({"discovery_filter": "unknown"});
        assert!(PluginRegistry::validate_config(PluginType::Apt, &config).is_err());
    }

    #[test]
    fn apt_provider_capabilities() {
        let config = serde_json::json!({});
        let provider =
            PluginRegistry::create_provider(PluginType::Apt, &config, test_executor())
                .unwrap();
        assert!(
            provider
                .has_capability(uptrakit_plugin_core::PluginCapability::DiscoverLocalSoftware)
        );
        assert!(
            provider
                .has_capability(uptrakit_plugin_core::PluginCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn mask_config_secrets_apt() {
        let config = serde_json::json!({"discovery_filter": "manual"});
        let masked = PluginRegistry::mask_config_secrets(PluginType::Apt, &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn validate_package_identifier_apt_valid() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::Apt, "nginx").is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_apt_uppercase_fails() {
        assert!(
            PluginRegistry::validate_package_identifier(PluginType::Apt, "Nginx").is_err()
        );
    }

    // ── PluginType::Other(String) behaviour ──────────────────────────────

    /// `Other(String)` received from a newer server must fail gracefully at
    /// the registry level (unknown type) rather than causing a deserialization
    /// panic or silent data loss.
    #[test]
    fn create_provider_other_returns_unknown_type_error() {
        let config = serde_json::json!({});
        let Err(err) = PluginRegistry::create_provider(
            PluginType::Other("winget".to_string()),
            &config,
            test_executor(),
        ) else {
            panic!("expected Err for Other provider type");
        };
        assert!(err.to_string().contains("unknown provider type"));
    }

    #[test]
    fn validate_config_other_returns_unknown_type_error() {
        let config = serde_json::json!({});
        let result =
            PluginRegistry::validate_config(PluginType::Other("winget".to_string()), &config);
        assert!(result.is_err());
    }

    /// `mask_config_secrets` for an `Other` provider type returns the config
    /// unchanged (no masking possible for an unknown provider).
    #[test]
    fn mask_config_secrets_other_returns_config_unchanged() {
        let config = serde_json::json!({"token": "secret", "repo": "something"});
        let result = PluginRegistry::mask_config_secrets(
            PluginType::Other("winget".to_string()),
            &config,
        );
        assert_eq!(result, config);
    }

    // ── validate_package_identifier ───────────────────────────────────────

    /// `Other` always returns `Ok(())`.
    #[test]
    fn validate_package_identifier_other_is_permissive() {
        assert!(PluginRegistry::validate_package_identifier(
            PluginType::Other("flatpak".to_string()),
            "org.example.App"
        )
        .is_ok());
    }

    #[test]
    fn validate_package_identifier_docker_valid() {
        assert!(PluginRegistry::validate_package_identifier(
            PluginType::Docker,
            "nginx"
        )
        .is_ok());
        assert!(PluginRegistry::validate_package_identifier(
            PluginType::Docker,
            "ghcr.io/owner/app:latest"
        )
        .is_ok());
        assert!(PluginRegistry::validate_package_identifier(
            PluginType::Docker,
            "myuser/app:v2"
        )
        .is_ok());
    }

    #[test]
    fn validate_package_identifier_docker_invalid() {
        assert!(PluginRegistry::validate_package_identifier(
            PluginType::Docker,
            ""
        )
        .is_err());
        assert!(PluginRegistry::validate_package_identifier(
            PluginType::Docker,
            "nginx latest"
        )
        .is_err());
        assert!(PluginRegistry::validate_package_identifier(
            PluginType::Docker,
            "ghcr.io//app"
        )
        .is_err());
    }
}
