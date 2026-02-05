use rootcause::report;

use uptrakit_provider_core::{Provider, ProviderType};
use uptrakit_provider_docker_registry::{
    DockerRegistryConfig, DockerRegistryLocalProvider, DockerRegistryProvider,
};
use uptrakit_provider_github::{GitHubConfig, GitHubLocalProvider, GitHubProvider};
use uptrakit_provider_proxmox_helper_scripts::ProxmoxHelperScriptsLocalProvider;

use crate::error::{RegistryError, Result};
use crate::secrets;

/// Provider registry for creating and validating providers.
///
/// This struct provides a centralized API for:
/// - Creating local and remote provider instances
/// - Validating provider configuration
/// - Masking and restoring secrets in configuration
pub struct ProviderRegistry;

impl ProviderRegistry {
    /// Create a local provider instance from provider type and config.
    ///
    /// # Arguments
    ///
    /// * `provider_type` - The type of provider to create
    /// * `package_identifier` - Provider-specific package identifier
    /// * `config` - Provider configuration as JSON
    ///
    /// # Returns
    ///
    /// A boxed `Provider` trait object on success, or a `RegistryError` on failure.
    pub fn create_local_provider(
        provider_type: ProviderType,
        package_identifier: &str,
        config: &serde_json::Value,
    ) -> Result<Box<dyn Provider>> {
        match provider_type {
            ProviderType::GithubReleases => {
                let github_config: GitHubConfig = serde_json::from_value(config.clone())
                    .map_err(|e| report!(RegistryError::ConfigParse(e)))?;
                let provider =
                    GitHubLocalProvider::new(github_config, package_identifier.to_string());
                Ok(Box::new(provider))
            }
            ProviderType::DockerRegistry => {
                let provider = DockerRegistryLocalProvider::new();
                Ok(Box::new(provider))
            }
            ProviderType::ProxmoxHelperScripts => {
                let provider =
                    ProxmoxHelperScriptsLocalProvider::new(package_identifier.to_string());
                Ok(Box::new(provider))
            }
        }
    }

    /// Create a remote provider instance from provider type and config.
    ///
    /// # Arguments
    ///
    /// * `provider_type` - The type of provider to create
    /// * `config` - Provider configuration as JSON
    ///
    /// # Returns
    ///
    /// A boxed `Provider` trait object on success, or a `RegistryError` on failure.
    pub fn create_remote_provider(
        provider_type: ProviderType,
        config: &serde_json::Value,
    ) -> Result<Box<dyn Provider>> {
        match provider_type {
            ProviderType::GithubReleases => {
                let github_config: GitHubConfig = serde_json::from_value(config.clone())
                    .map_err(|e| report!(RegistryError::ConfigParse(e)))?;
                let provider = GitHubProvider::new(github_config)
                    .map_err(|e| report!(RegistryError::Instantiation(e.to_string())))?;
                Ok(Box::new(provider))
            }
            ProviderType::DockerRegistry => {
                let docker_config: DockerRegistryConfig = serde_json::from_value(config.clone())
                    .map_err(|e| report!(RegistryError::ConfigParse(e)))?;
                let provider = DockerRegistryProvider::new(docker_config)
                    .map_err(|e| report!(RegistryError::Instantiation(e.to_string())))?;
                Ok(Box::new(provider))
            }
            ProviderType::ProxmoxHelperScripts => {
                // Proxmox Helper Scripts doesn't have a remote provider - it's local-only
                Err(report!(RegistryError::ConfigValidation(
                    "proxmox_helper_scripts does not support remote operations".to_string()
                )))
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
                let github_config: GitHubConfig = serde_json::from_value(config.clone())
                    .map_err(|e| report!(RegistryError::ConfigParse(e)))?;
                github_config
                    .validate()
                    .map_err(|e| report!(RegistryError::ConfigValidation(e.to_string())))?;
                Ok(())
            }
            ProviderType::DockerRegistry => {
                let docker_config: DockerRegistryConfig = serde_json::from_value(config.clone())
                    .map_err(|e| report!(RegistryError::ConfigParse(e)))?;
                docker_config
                    .validate()
                    .map_err(|e| report!(RegistryError::ConfigValidation(e.to_string())))?;
                Ok(())
            }
            ProviderType::ProxmoxHelperScripts => {
                // No validation yet for this provider type
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
    /// Replaces sensitive fields (tokens, passwords) with a sentinel value.
    ///
    /// # Arguments
    ///
    /// * `provider_type` - The type of provider
    /// * `config` - Provider configuration as JSON
    ///
    /// # Returns
    ///
    /// A new JSON value with sensitive fields masked.
    pub fn mask_secrets(
        provider_type: ProviderType,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        secrets::mask_secrets(&provider_type.to_string(), config)
    }

    /// Mask secrets in provider configuration JSON (string type version).
    pub fn mask_secrets_str(provider_type: &str, config: &serde_json::Value) -> serde_json::Value {
        secrets::mask_secrets(provider_type, config)
    }

    /// Restore masked secrets from existing configuration.
    ///
    /// When the incoming config contains the mask sentinel value, the corresponding
    /// value from the existing config is restored.
    ///
    /// # Arguments
    ///
    /// * `provider_type` - The type of provider
    /// * `incoming` - Incoming configuration (modified in place)
    /// * `existing` - Existing configuration with real secrets
    pub fn restore_secrets(
        provider_type: ProviderType,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        secrets::restore_secrets(&provider_type.to_string(), incoming, existing)
    }

    /// Restore masked secrets from existing configuration (string type version).
    pub fn restore_secrets_str(
        provider_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        secrets::restore_secrets(provider_type, incoming, existing)
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

    #[tokio::test]
    async fn create_local_provider_github() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let provider =
            ProviderRegistry::create_local_provider(ProviderType::GithubReleases, "test", &config);
        assert!(provider.is_ok());
    }

    #[tokio::test]
    async fn create_local_provider_docker() {
        let config = serde_json::json!({});
        let provider =
            ProviderRegistry::create_local_provider(ProviderType::DockerRegistry, "nginx", &config);
        assert!(provider.is_ok());
    }

    #[tokio::test]
    async fn create_local_provider_proxmox() {
        let config = serde_json::json!({});
        let provider = ProviderRegistry::create_local_provider(
            ProviderType::ProxmoxHelperScripts,
            "test-script",
            &config,
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn create_remote_provider_github() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let provider =
            ProviderRegistry::create_remote_provider(ProviderType::GithubReleases, &config);
        assert!(provider.is_ok());
    }

    #[test]
    fn create_remote_provider_docker() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        let provider =
            ProviderRegistry::create_remote_provider(ProviderType::DockerRegistry, &config);
        assert!(provider.is_ok());
    }

    #[test]
    fn create_remote_provider_proxmox_fails() {
        let config = serde_json::json!({});
        let provider =
            ProviderRegistry::create_remote_provider(ProviderType::ProxmoxHelperScripts, &config);
        assert!(provider.is_err());
    }

    #[test]
    fn mask_secrets_github() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "auth_token": "ghp_secret"
        });
        let masked = ProviderRegistry::mask_secrets(ProviderType::GithubReleases, &config);
        assert_eq!(masked["auth_token"], "***");
        assert_eq!(masked["owner"], "octocat");
    }

    #[test]
    fn restore_secrets_github() {
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
        ProviderRegistry::restore_secrets(ProviderType::GithubReleases, &mut incoming, &existing);
        assert_eq!(incoming["auth_token"], "ghp_real_token");
    }
}
