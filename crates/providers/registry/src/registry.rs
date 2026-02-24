use std::sync::Arc;

use rootcause::prelude::*;

use uptrakit_command::CommandExecutor;
use uptrakit_provider_core::{Provider, ProviderType, SecretMasking, SudoCommandEntry};
use uptrakit_provider_apt::{AptConfig, AptProvider};
use uptrakit_provider_docker::{DockerConfig, DockerProvider};
use uptrakit_provider_github::{GitHubConfig, GitHubProvider};
use uptrakit_provider_homebrew::{HomebrewConfig, HomebrewProvider};
use uptrakit_provider_proxmox_helper_scripts::{
    ProxmoxHelperScriptsConfig, ProxmoxHelperScriptsProvider,
};

use crate::error::{RegistryError, Result};

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

/// Generates the six `ProviderRegistry` dispatch methods from a single
/// declaration list, eliminating manually-maintained match arms.
macro_rules! register_providers {
    ($(
        $variant:ident => { config: $config:ty, provider: $provider:ty }
    ),+ $(,)?) => {
        impl ProviderRegistry {
            /// Create a provider instance from provider type, config, and executor.
            ///
            /// Deserializes the config, validates it, and constructs the provider.
            /// All providers follow the same pattern: deserialize → validate →
            /// construct.
            pub fn create_provider(
                provider_type: ProviderType,
                config: &serde_json::Value,
                executor: Arc<dyn CommandExecutor>,
            ) -> Result<Box<dyn Provider>> {
                match provider_type {
                    $(
                        ProviderType::$variant => {
                            let typed_config: $config =
                                serde_json::from_value(config.clone()).context_to()?;
                            typed_config
                                .validate()
                                .map_err(|e| report!(RegistryError::ConfigValidation(e.to_string())))?;
                            let provider = <$provider>::new(typed_config, executor)
                                .map_err(|e| report!(RegistryError::Instantiation(e.to_string())))?;
                            Ok(Box::new(provider))
                        }
                    )+
                    _ => Err(report!(RegistryError::UnknownProviderType(format!(
                        "{provider_type}"
                    )))),
                }
            }

            /// Validate provider configuration JSON.
            pub fn validate_config(
                provider_type: ProviderType,
                config: &serde_json::Value,
            ) -> Result<()> {
                match provider_type {
                    $(
                        ProviderType::$variant => {
                            let typed_config: $config =
                                serde_json::from_value(config.clone()).context_to()?;
                            typed_config
                                .validate()
                                .map_err(|e| report!(RegistryError::ConfigValidation(e.to_string())))?;
                            Ok(())
                        }
                    )+
                    _ => Err(report!(RegistryError::UnknownProviderType(format!(
                        "{provider_type}"
                    )))),
                }
            }

            /// Mask secrets in provider configuration JSON for API responses.
            ///
            /// Deserializes config, calls [`SecretMasking::with_secrets_masked()`],
            /// and serializes back. Unknown provider types are returned unchanged.
            pub fn mask_config_secrets(
                provider_type: ProviderType,
                config: &serde_json::Value,
            ) -> serde_json::Value {
                match provider_type {
                    $(
                        ProviderType::$variant => mask_secrets_for::<$config>(config),
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
                provider_type: ProviderType,
                incoming: &mut serde_json::Value,
                existing: &serde_json::Value,
            ) {
                match provider_type {
                    $(
                        ProviderType::$variant => {
                            restore_secrets_for::<$config>(incoming, existing);
                        }
                    )+
                    _ => {}
                }
            }

            /// Create a provider instance for autodiscovery, bypassing `validate()`.
            ///
            /// Discovery can proceed with an empty/minimal config — e.g. a
            /// `ProxmoxHelperScriptsConfig` with `script_url = ""` can still
            /// run `discover_software()` even though `validate()` would reject it.
            pub fn create_provider_for_discovery(
                provider_type: ProviderType,
                config: &serde_json::Value,
                executor: Arc<dyn CommandExecutor>,
            ) -> Result<Box<dyn Provider>> {
                match provider_type {
                    $(
                        ProviderType::$variant => {
                            let typed_config: $config =
                                serde_json::from_value(config.clone()).context_to()?;
                            // No validate() — discovery can proceed with an empty/minimal config.
                            let provider = <$provider>::new(typed_config, executor)
                                .map_err(|e| report!(RegistryError::Instantiation(e.to_string())))?;
                            Ok(Box::new(provider))
                        }
                    )+
                    _ => Err(report!(RegistryError::UnknownProviderType(format!(
                        "{provider_type}"
                    )))),
                }
            }

            /// Returns all provider types that have the `DiscoverLocalSoftware` capability.
            ///
            /// Auto-derived from the macro registration — no manual list needed.
            pub fn discovery_provider_types() -> Vec<ProviderType> {
                let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
                let empty = serde_json::Value::Object(serde_json::Map::new());
                let mut result = Vec::new();
                $(
                    if let Ok(p) = Self::create_provider_for_discovery(
                        ProviderType::$variant, &empty, executor.clone())
                    {
                        if p.has_capability(uptrakit_provider_core::ProviderCapability::DiscoverLocalSoftware) {
                            result.push(ProviderType::$variant);
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
            pub fn all_required_sudo_commands() -> Vec<(ProviderType, Vec<SudoCommandEntry>)> {
                let executor = Arc::new(uptrakit_command::LocalCommandExecutor) as Arc<dyn CommandExecutor>;
                let empty = serde_json::Value::Object(serde_json::Map::new());
                let mut result = Vec::new();
                $(
                    if let Ok(p) = Self::create_provider_for_discovery(
                        ProviderType::$variant, &empty, executor.clone())
                    {
                        let entries = p.required_sudo_commands();
                        if !entries.is_empty() {
                            result.push((ProviderType::$variant, entries));
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
/// generated by the [`register_providers!`] macro from a single declaration.
/// To add a new provider, add one line to the macro invocation below.
pub struct ProviderRegistry;

register_providers! {
    GithubReleases       => { config: GitHubConfig,                   provider: GitHubProvider },
    Docker               => { config: DockerConfig,                   provider: DockerProvider },
    ProxmoxHelperScripts => { config: ProxmoxHelperScriptsConfig,     provider: ProxmoxHelperScriptsProvider },
    Homebrew             => { config: HomebrewConfig,                 provider: HomebrewProvider },
    Apt                  => { config: AptConfig,                      provider: AptProvider },
}

impl ProviderRegistry {
    /// Validate a package identifier for the given provider type.
    ///
    /// Returns `Ok(())` for provider types that have no identifier constraints.
    /// Returns `Err(message)` when the identifier violates provider-specific rules.
    pub fn validate_package_identifier(
        provider_type: ProviderType,
        value: &str,
    ) -> std::result::Result<(), String> {
        match provider_type {
            ProviderType::Docker => uptrakit_provider_docker::validate_identifier(value),
            ProviderType::Homebrew => uptrakit_provider_homebrew::validate_identifier(value),
            ProviderType::Apt => uptrakit_provider_apt::validate_identifier(value),
            _ => Ok(()),
        }
    }

    /// Validate provider configuration from string type.
    ///
    /// This is a convenience method that accepts a string provider type.
    pub fn validate_config_str(provider_type: &str, config: &serde_json::Value) -> Result<()> {
        let pt: ProviderType = provider_type.parse().map_err(|_| {
            report!(RegistryError::UnknownProviderType(
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
        let Ok(pt) = provider_type.parse::<ProviderType>() else {
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
        let Ok(pt) = provider_type.parse::<ProviderType>() else {
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
            "github_releases".parse::<ProviderType>().ok(),
            Some(ProviderType::GithubReleases)
        );
        assert_eq!(
            "docker".parse::<ProviderType>().ok(),
            Some(ProviderType::Docker)
        );
        assert_eq!(
            "proxmox_helper_scripts".parse::<ProviderType>().ok(),
            Some(ProviderType::ProxmoxHelperScripts)
        );
        assert_eq!(
            "homebrew".parse::<ProviderType>().ok(),
            Some(ProviderType::Homebrew)
        );
        assert_eq!(
            "apt".parse::<ProviderType>().ok(),
            Some(ProviderType::Apt)
        );
        assert!("unknown".parse::<ProviderType>().is_err());
        // Old wire string is no longer a known type
        assert!("docker_registry".parse::<ProviderType>().is_err());
    }

    #[test]
    fn validate_valid_github_config() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        assert!(ProviderRegistry::validate_config(ProviderType::GithubReleases, &config).is_ok());
    }

    #[test]
    fn validate_invalid_github_config() {
        let config = serde_json::json!({
            "owner": "",
            "repo": "hello-world"
        });
        assert!(ProviderRegistry::validate_config(ProviderType::GithubReleases, &config).is_err());
    }

    #[test]
    fn validate_valid_docker_config() {
        // Empty config is valid for Docker (no required fields)
        let config = serde_json::json!({});
        assert!(ProviderRegistry::validate_config(ProviderType::Docker, &config).is_ok());
    }

    #[test]
    fn validate_invalid_docker_config_zero_page_size() {
        let config = serde_json::json!({ "page_size": 0 });
        assert!(ProviderRegistry::validate_config(ProviderType::Docker, &config).is_err());
    }

    #[test]
    fn validate_proxmox_helper_scripts_config() {
        let config = serde_json::json!({"script_url": "https://example.com/update.sh"});
        assert!(
            ProviderRegistry::validate_config(ProviderType::ProxmoxHelperScripts, &config).is_ok()
        );
    }

    #[test]
    fn validate_proxmox_helper_scripts_empty_url_fails() {
        let config = serde_json::json!({"script_url": ""});
        assert!(
            ProviderRegistry::validate_config(ProviderType::ProxmoxHelperScripts, &config).is_err()
        );
    }

    #[test]
    fn validate_config_str_valid() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        assert!(ProviderRegistry::validate_config_str("github_releases", &config).is_ok());
    }

    #[test]
    fn validate_config_str_unknown_type() {
        let config = serde_json::json!({});
        let result = ProviderRegistry::validate_config_str("unknown", &config);
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
        let provider = ProviderRegistry::create_provider(
            ProviderType::GithubReleases,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_docker() {
        // Empty config is valid
        let config = serde_json::json!({});
        let provider = ProviderRegistry::create_provider(
            ProviderType::Docker,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_for_discovery_docker() {
        let config = serde_json::json!({});
        let provider = ProviderRegistry::create_provider_for_discovery(
            ProviderType::Docker,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_proxmox() {
        let config = serde_json::json!({"script_url": "https://example.com/update.sh"});
        let provider = ProviderRegistry::create_provider(
            ProviderType::ProxmoxHelperScripts,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_proxmox_with_github() {
        let config = serde_json::json!({
            "script_url": "https://example.com/update.sh",
            "github": {
                "owner": "BookLore",
                "repo": "BookLore"
            }
        });
        let provider = ProviderRegistry::create_provider(
            ProviderType::ProxmoxHelperScripts,
            &config,
            test_executor(),
        );
        assert!(provider.is_ok());
        let provider = provider.expect("create");
        assert!(
            provider
                .has_capability(uptrakit_provider_core::ProviderCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn create_provider_proxmox_with_invalid_github_fails() {
        let config = serde_json::json!({
            "script_url": "https://example.com/update.sh",
            "github": {
                "owner": "",
                "repo": "BookLore"
            }
        });
        let result = ProviderRegistry::create_provider(
            ProviderType::ProxmoxHelperScripts,
            &config,
            test_executor(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn mask_config_secrets_proxmox_with_github() {
        let config = serde_json::json!({
            "script_url": "https://example.com/update.sh",
            "github": {
                "owner": "owner",
                "repo": "repo",
                "auth_token": "ghp_secret"
            }
        });
        let masked =
            ProviderRegistry::mask_config_secrets(ProviderType::ProxmoxHelperScripts, &config);
        assert_eq!(masked["github"]["auth_token"], "***");
        assert_eq!(masked["github"]["owner"], "owner");
    }

    #[test]
    fn restore_config_secrets_proxmox_with_github() {
        let mut incoming = serde_json::json!({
            "script_url": "https://example.com/update.sh",
            "github": {
                "owner": "owner",
                "repo": "repo",
                "auth_token": "***"
            }
        });
        let existing = serde_json::json!({
            "script_url": "https://example.com/update.sh",
            "github": {
                "owner": "owner",
                "repo": "repo",
                "auth_token": "ghp_real_token"
            }
        });
        ProviderRegistry::restore_config_secrets(
            ProviderType::ProxmoxHelperScripts,
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["github"]["auth_token"], "ghp_real_token");
    }

    #[test]
    fn create_provider_homebrew() {
        let config = serde_json::json!({});
        let provider =
            ProviderRegistry::create_provider(ProviderType::Homebrew, &config, test_executor());
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_homebrew_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        let provider =
            ProviderRegistry::create_provider(ProviderType::Homebrew, &config, test_executor());
        assert!(provider.is_ok());
    }

    #[test]
    fn validate_homebrew_config() {
        let config = serde_json::json!({});
        assert!(ProviderRegistry::validate_config(ProviderType::Homebrew, &config).is_ok());
    }

    #[test]
    fn validate_homebrew_config_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        assert!(ProviderRegistry::validate_config(ProviderType::Homebrew, &config).is_ok());
    }

    #[test]
    fn validate_homebrew_config_invalid_package_type() {
        let config = serde_json::json!({"package_type": "invalid"});
        assert!(ProviderRegistry::validate_config(ProviderType::Homebrew, &config).is_err());
    }

    #[test]
    fn homebrew_provider_capabilities() {
        let config = serde_json::json!({});
        let provider =
            ProviderRegistry::create_provider(ProviderType::Homebrew, &config, test_executor())
                .unwrap();
        assert!(
            provider
                .has_capability(uptrakit_provider_core::ProviderCapability::DiscoverLocalSoftware)
        );
        assert!(
            provider
                .has_capability(uptrakit_provider_core::ProviderCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn docker_provider_capabilities() {
        let config = serde_json::json!({});
        let provider =
            ProviderRegistry::create_provider(ProviderType::Docker, &config, test_executor())
                .unwrap();
        assert!(
            provider
                .has_capability(uptrakit_provider_core::ProviderCapability::DiscoverLocalSoftware)
        );
        assert!(
            !provider
                .has_capability(uptrakit_provider_core::ProviderCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn discovery_provider_types_includes_docker() {
        let types = ProviderRegistry::discovery_provider_types();
        assert!(
            types.contains(&ProviderType::Docker),
            "Docker should be in discovery_provider_types()"
        );
    }

    #[test]
    fn all_required_sudo_commands_includes_apt() {
        let entries = ProviderRegistry::all_required_sudo_commands();
        let apt_entry = entries
            .iter()
            .find(|(pt, _)| *pt == ProviderType::Apt)
            .expect("Apt should have sudo command entries");
        assert!(!apt_entry.1.is_empty());
        assert_eq!(apt_entry.1[0].command, "apt-get");
    }

    #[test]
    fn all_required_sudo_commands_no_duplicates_per_provider() {
        let entries = ProviderRegistry::all_required_sudo_commands();
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
        let github = ProviderRegistry::create_provider(
            ProviderType::GithubReleases,
            &github_config,
            test_executor(),
        )
        .expect("create github");
        assert_eq!(github.provider_type(), ProviderType::GithubReleases);

        let docker_config = serde_json::json!({});
        let docker = ProviderRegistry::create_provider(
            ProviderType::Docker,
            &docker_config,
            test_executor(),
        )
        .expect("create docker");
        assert_eq!(docker.provider_type(), ProviderType::Docker);

        let proxmox_config = serde_json::json!({"script_url": "https://example.com/update.sh"});
        let proxmox = ProviderRegistry::create_provider(
            ProviderType::ProxmoxHelperScripts,
            &proxmox_config,
            test_executor(),
        )
        .expect("create proxmox");
        assert_eq!(proxmox.provider_type(), ProviderType::ProxmoxHelperScripts);

        let homebrew_config = serde_json::json!({});
        let homebrew = ProviderRegistry::create_provider(
            ProviderType::Homebrew,
            &homebrew_config,
            test_executor(),
        )
        .expect("create homebrew");
        assert_eq!(homebrew.provider_type(), ProviderType::Homebrew);

        let apt_config = serde_json::json!({});
        let apt =
            ProviderRegistry::create_provider(ProviderType::Apt, &apt_config, test_executor())
                .expect("create apt");
        assert_eq!(apt.provider_type(), ProviderType::Apt);
    }

    #[test]
    fn mask_config_secrets_homebrew() {
        let config = serde_json::json!({"package_type": "formula"});
        let masked = ProviderRegistry::mask_config_secrets(ProviderType::Homebrew, &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn mask_config_secrets_github() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_secret"
        });
        let masked = ProviderRegistry::mask_config_secrets(ProviderType::GithubReleases, &config);
        assert_eq!(masked["auth_token"], "***");
        assert_eq!(masked["owner"], "octocat");
    }

    #[test]
    fn mask_config_secrets_github_always_shows_field() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let masked = ProviderRegistry::mask_config_secrets(ProviderType::GithubReleases, &config);
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
        ProviderRegistry::restore_config_secrets(
            ProviderType::GithubReleases,
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["auth_token"], "ghp_real_token");
    }

    #[test]
    fn create_provider_apt() {
        let config = serde_json::json!({});
        let provider =
            ProviderRegistry::create_provider(ProviderType::Apt, &config, test_executor());
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_apt_all_filter() {
        let config = serde_json::json!({"discovery_filter": "all"});
        let provider =
            ProviderRegistry::create_provider(ProviderType::Apt, &config, test_executor());
        assert!(provider.is_ok());
    }

    #[test]
    fn validate_apt_config() {
        let config = serde_json::json!({});
        assert!(ProviderRegistry::validate_config(ProviderType::Apt, &config).is_ok());
    }

    #[test]
    fn validate_apt_config_invalid_filter_fails() {
        let config = serde_json::json!({"discovery_filter": "unknown"});
        assert!(ProviderRegistry::validate_config(ProviderType::Apt, &config).is_err());
    }

    #[test]
    fn apt_provider_capabilities() {
        let config = serde_json::json!({});
        let provider =
            ProviderRegistry::create_provider(ProviderType::Apt, &config, test_executor())
                .unwrap();
        assert!(
            provider
                .has_capability(uptrakit_provider_core::ProviderCapability::DiscoverLocalSoftware)
        );
        assert!(
            provider
                .has_capability(uptrakit_provider_core::ProviderCapability::RefreshPackageIndex)
        );
    }

    #[test]
    fn mask_config_secrets_apt() {
        let config = serde_json::json!({"discovery_filter": "manual"});
        let masked = ProviderRegistry::mask_config_secrets(ProviderType::Apt, &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn validate_package_identifier_apt_valid() {
        assert!(
            ProviderRegistry::validate_package_identifier(ProviderType::Apt, "nginx").is_ok()
        );
    }

    #[test]
    fn validate_package_identifier_apt_uppercase_fails() {
        assert!(
            ProviderRegistry::validate_package_identifier(ProviderType::Apt, "Nginx").is_err()
        );
    }

    // ── ProviderType::Other(String) behaviour ──────────────────────────────

    /// `Other(String)` received from a newer server must fail gracefully at
    /// the registry level (unknown type) rather than causing a deserialization
    /// panic or silent data loss.
    #[test]
    fn create_provider_other_returns_unknown_type_error() {
        let config = serde_json::json!({});
        let Err(err) = ProviderRegistry::create_provider(
            ProviderType::Other("winget".to_string()),
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
            ProviderRegistry::validate_config(ProviderType::Other("winget".to_string()), &config);
        assert!(result.is_err());
    }

    /// `mask_config_secrets` for an `Other` provider type returns the config
    /// unchanged (no masking possible for an unknown provider).
    #[test]
    fn mask_config_secrets_other_returns_config_unchanged() {
        let config = serde_json::json!({"token": "secret", "repo": "something"});
        let result = ProviderRegistry::mask_config_secrets(
            ProviderType::Other("winget".to_string()),
            &config,
        );
        assert_eq!(result, config);
    }

    // ── validate_package_identifier ───────────────────────────────────────

    /// `Other` always returns `Ok(())`.
    #[test]
    fn validate_package_identifier_other_is_permissive() {
        assert!(ProviderRegistry::validate_package_identifier(
            ProviderType::Other("flatpak".to_string()),
            "org.example.App"
        )
        .is_ok());
    }

    #[test]
    fn validate_package_identifier_docker_valid() {
        assert!(ProviderRegistry::validate_package_identifier(
            ProviderType::Docker,
            "nginx"
        )
        .is_ok());
        assert!(ProviderRegistry::validate_package_identifier(
            ProviderType::Docker,
            "ghcr.io/owner/app:latest"
        )
        .is_ok());
        assert!(ProviderRegistry::validate_package_identifier(
            ProviderType::Docker,
            "myuser/app:v2"
        )
        .is_ok());
    }

    #[test]
    fn validate_package_identifier_docker_invalid() {
        assert!(ProviderRegistry::validate_package_identifier(
            ProviderType::Docker,
            ""
        )
        .is_err());
        assert!(ProviderRegistry::validate_package_identifier(
            ProviderType::Docker,
            "nginx latest"
        )
        .is_err());
        assert!(ProviderRegistry::validate_package_identifier(
            ProviderType::Docker,
            "ghcr.io//app"
        )
        .is_err());
    }
}
