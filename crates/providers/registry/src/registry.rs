use rootcause::prelude::*;

use uptrakit_provider_core::{Provider, ProviderType};
use uptrakit_provider_docker_registry::{DockerRegistryConfig, DockerRegistryProvider};
use uptrakit_provider_github::{GitHubConfig, GitHubProvider};
use uptrakit_provider_homebrew::{HomebrewConfig, HomebrewProvider};
use uptrakit_provider_proxmox_helper_scripts::ProxmoxHelperScriptsProvider;

use crate::error::{RegistryError, Result};

/// Provider registry for creating and validating providers.
///
/// This struct provides a centralized API for:
/// - Creating provider instances from type and config
/// - Validating provider configuration
/// - Masking and restoring secrets in configuration
pub struct ProviderRegistry;

impl ProviderRegistry {
    /// Create a provider instance from provider type and config.
    ///
    /// # Arguments
    ///
    /// * `provider_type` - The type of provider to create
    /// * `config` - Provider configuration as JSON
    ///
    /// # Returns
    ///
    /// A boxed `Provider` trait object on success, or a `RegistryError` on failure.
    pub fn create_provider(
        provider_type: ProviderType,
        config: &serde_json::Value,
    ) -> Result<Box<dyn Provider>> {
        match provider_type {
            ProviderType::GithubReleases => {
                let github_config: GitHubConfig =
                    serde_json::from_value(config.clone()).context_to()?;
                let provider = GitHubProvider::new(github_config)
                    .map_err(|e| report!(RegistryError::Instantiation(e.to_string())))?;
                Ok(Box::new(provider))
            }
            ProviderType::DockerRegistry => {
                let docker_config: DockerRegistryConfig =
                    serde_json::from_value(config.clone()).context_to()?;
                let provider = DockerRegistryProvider::new(docker_config)
                    .map_err(|e| report!(RegistryError::Instantiation(e.to_string())))?;
                Ok(Box::new(provider))
            }
            ProviderType::ProxmoxHelperScripts => {
                let provider = ProxmoxHelperScriptsProvider::new();
                Ok(Box::new(provider))
            }
            ProviderType::Homebrew => {
                let homebrew_config: HomebrewConfig =
                    serde_json::from_value(config.clone()).context_to()?;
                let provider = HomebrewProvider::new(homebrew_config);
                Ok(Box::new(provider))
            }
        }
    }

    /// Validate provider configuration JSON.
    ///
    /// # Arguments
    ///
    /// * `provider_type` - The type of provider
    /// * `config` - Provider configuration as JSON
    ///
    /// # Returns
    ///
    /// `Ok(())` if configuration is valid, or a `RegistryError` describing the validation failure.
    pub fn validate_config(provider_type: ProviderType, config: &serde_json::Value) -> Result<()> {
        match provider_type {
            ProviderType::GithubReleases => {
                let github_config: GitHubConfig =
                    serde_json::from_value(config.clone()).context_to()?;
                github_config
                    .validate()
                    .map_err(|e| report!(RegistryError::ConfigValidation(e.to_string())))?;
                Ok(())
            }
            ProviderType::DockerRegistry => {
                let docker_config: DockerRegistryConfig =
                    serde_json::from_value(config.clone()).context_to()?;
                docker_config
                    .validate()
                    .map_err(|e| report!(RegistryError::ConfigValidation(e.to_string())))?;
                Ok(())
            }
            ProviderType::ProxmoxHelperScripts => {
                // No validation yet for this provider type
                Ok(())
            }
            ProviderType::Homebrew => {
                let homebrew_config: HomebrewConfig =
                    serde_json::from_value(config.clone()).context_to()?;
                homebrew_config
                    .validate()
                    .map_err(|e| report!(RegistryError::ConfigValidation(e)))?;
                Ok(())
            }
        }
    }

    /// Validate provider configuration from string type.
    ///
    /// This is a convenience method that accepts a string provider type.
    ///
    /// # Arguments
    ///
    /// * `provider_type` - The provider type as a string (e.g., "github_releases")
    /// * `config` - Provider configuration as JSON
    ///
    /// # Returns
    ///
    /// `Ok(())` if configuration is valid, or a `RegistryError` describing the validation failure.
    pub fn validate_config_str(provider_type: &str, config: &serde_json::Value) -> Result<()> {
        let pt: ProviderType = serde_json::from_value(serde_json::Value::String(
            provider_type.to_string(),
        ))
        .map_err(|_| {
            report!(RegistryError::UnknownProviderType(
                provider_type.to_string()
            ))
        })?;

        Self::validate_config(pt, config)
    }

    /// Mask secrets in provider configuration JSON for API responses.
    ///
    /// Deserializes config, calls the typed `with_secrets_masked()` method, and
    /// serializes back. Unknown provider types are returned unchanged.
    pub fn mask_config_secrets(
        provider_type: ProviderType,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        match provider_type {
            ProviderType::GithubReleases => {
                let Ok(cfg) = serde_json::from_value::<GitHubConfig>(config.clone()) else {
                    return config.clone();
                };
                serde_json::to_value(cfg.with_secrets_masked()).unwrap_or_else(|_| config.clone())
            }
            ProviderType::DockerRegistry => {
                let Ok(cfg) = serde_json::from_value::<DockerRegistryConfig>(config.clone()) else {
                    return config.clone();
                };
                serde_json::to_value(cfg.with_secrets_masked()).unwrap_or_else(|_| config.clone())
            }
            ProviderType::ProxmoxHelperScripts => config.clone(),
            // Homebrew has no secrets to mask
            ProviderType::Homebrew => config.clone(),
        }
    }

    /// Mask secrets in provider configuration JSON (string type version).
    pub fn mask_config_secrets_str(
        provider_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        let Some(pt) = Self::parse_provider_type(provider_type) else {
            return config.clone();
        };
        Self::mask_config_secrets(pt, config)
    }

    /// Restore masked secrets from existing configuration.
    ///
    /// Deserializes both incoming and existing configs, calls the typed
    /// `restore_secrets_from()` method, and writes back to `incoming`.
    pub fn restore_config_secrets(
        provider_type: ProviderType,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        match provider_type {
            ProviderType::GithubReleases => {
                let (Ok(mut inc), Ok(ex)) = (
                    serde_json::from_value::<GitHubConfig>(incoming.clone()),
                    serde_json::from_value::<GitHubConfig>(existing.clone()),
                ) else {
                    return;
                };
                inc.restore_secrets_from(&ex);
                if let Ok(v) = serde_json::to_value(&inc) {
                    *incoming = v;
                }
            }
            ProviderType::DockerRegistry => {
                let (Ok(mut inc), Ok(ex)) = (
                    serde_json::from_value::<DockerRegistryConfig>(incoming.clone()),
                    serde_json::from_value::<DockerRegistryConfig>(existing.clone()),
                ) else {
                    return;
                };
                inc.restore_secrets_from(&ex);
                if let Ok(v) = serde_json::to_value(&inc) {
                    *incoming = v;
                }
            }
            ProviderType::ProxmoxHelperScripts => {}
            // Homebrew has no secrets to restore
            ProviderType::Homebrew => {}
        }
    }

    /// Restore masked secrets from existing configuration (string type version).
    pub fn restore_config_secrets_str(
        provider_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        let Some(pt) = Self::parse_provider_type(provider_type) else {
            return;
        };
        Self::restore_config_secrets(pt, incoming, existing)
    }

    /// Parse a provider type string into a `ProviderType` enum.
    ///
    /// # Arguments
    ///
    /// * `s` - The provider type string
    ///
    /// # Returns
    ///
    /// `Some(ProviderType)` if the string is valid, or `None` if unknown.
    pub fn parse_provider_type(s: &str) -> Option<ProviderType> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_provider_types() {
        assert_eq!(
            ProviderRegistry::parse_provider_type("github_releases"),
            Some(ProviderType::GithubReleases)
        );
        assert_eq!(
            ProviderRegistry::parse_provider_type("docker_registry"),
            Some(ProviderType::DockerRegistry)
        );
        assert_eq!(
            ProviderRegistry::parse_provider_type("proxmox_helper_scripts"),
            Some(ProviderType::ProxmoxHelperScripts)
        );
        assert_eq!(
            ProviderRegistry::parse_provider_type("homebrew"),
            Some(ProviderType::Homebrew)
        );
        assert!(ProviderRegistry::parse_provider_type("unknown").is_none());
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
    fn validate_valid_docker_registry_config() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        assert!(ProviderRegistry::validate_config(ProviderType::DockerRegistry, &config).is_ok());
    }

    #[test]
    fn validate_invalid_docker_registry_config() {
        let config = serde_json::json!({
            "image": ""
        });
        assert!(ProviderRegistry::validate_config(ProviderType::DockerRegistry, &config).is_err());
    }

    #[test]
    fn validate_proxmox_helper_scripts_config() {
        let config = serde_json::json!({});
        assert!(
            ProviderRegistry::validate_config(ProviderType::ProxmoxHelperScripts, &config).is_ok()
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
        let provider = ProviderRegistry::create_provider(ProviderType::GithubReleases, &config);
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_docker() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        let provider = ProviderRegistry::create_provider(ProviderType::DockerRegistry, &config);
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_proxmox() {
        let config = serde_json::json!({});
        let provider =
            ProviderRegistry::create_provider(ProviderType::ProxmoxHelperScripts, &config);
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_homebrew() {
        let config = serde_json::json!({});
        let provider = ProviderRegistry::create_provider(ProviderType::Homebrew, &config);
        assert!(provider.is_ok());
    }

    #[test]
    fn create_provider_homebrew_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        let provider = ProviderRegistry::create_provider(ProviderType::Homebrew, &config);
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
        let provider = ProviderRegistry::create_provider(ProviderType::Homebrew, &config).unwrap();
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
}
