//! Provider Registry for Uptrakit
//!
//! This crate provides a centralized registry for provider operations:
//!
//! - **Provider creation**: Create provider instances from configuration and executor
//! - **Configuration validation**: Validate provider-specific configuration JSON
//! - **Secret management**: Mask and restore sensitive fields in configuration
//!
//! # Example
//!
//! ```ignore
//! use std::sync::Arc;
//! use uptrakit_provider_registry::{ProviderRegistry, ProviderType, LocalCommandExecutor};
//!
//! // Validate configuration
//! let config = serde_json::json!({
//!     "owner": "octocat",
//!     "repo": "hello-world"
//! });
//! ProviderRegistry::validate_config(ProviderType::GithubReleases, &config)?;
//!
//! // Create a provider with a local executor
//! let executor = Arc::new(LocalCommandExecutor);
//! let provider = ProviderRegistry::create_provider(
//!     ProviderType::GithubReleases,
//!     &config,
//!     executor,
//! )?;
//!
//! // Detect installed version
//! let version = provider.detect_installed_version("example").await?;
//! ```

pub mod error;
pub mod registry;

pub use error::{RegistryError, Result};
pub use registry::ProviderRegistry;

// Re-export commonly used types for provider crate convenience
pub use uptrakit_provider_core::{Provider, ProviderCapability};
pub use uptrakit_shared_types::ProviderType;

// Re-export executor types for downstream convenience
pub use uptrakit_command::{CommandExecutor, LocalCommandExecutor};

/// Abstraction over the provider registry operations needed by the web API.
///
/// Defines the three operations used when persisting and returning provider
/// configurations over the REST API: config validation, secret masking for
/// API responses, and secret restoration on update. Implemented by
/// [`ProviderRegistry`].
///
/// Storing this trait in `AppState` as `Arc<dyn ProviderOps>` rather than
/// referencing `ProviderRegistry` directly decouples route handlers and query
/// helpers from the concrete registry, making them testable in isolation.
pub trait ProviderOps: Send + Sync + 'static {
    /// Validate provider configuration JSON for the given string provider type.
    fn validate_config_str(&self, provider_type: &str, config: &serde_json::Value) -> Result<()>;

    /// Mask secrets in provider configuration JSON for an API response.
    ///
    /// Returns the config with all secret fields replaced by `"***"`.
    /// Unknown provider types are returned unchanged.
    fn mask_config_secrets_str(
        &self,
        provider_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value;

    /// Restore masked secrets from an existing configuration.
    ///
    /// Fields in `incoming` that equal `"***"` are replaced with the
    /// corresponding values from `existing`. Non-masked fields are left
    /// untouched.
    fn restore_config_secrets_str(
        &self,
        provider_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    );

    /// Returns all provider types that have the `DiscoverLocalSoftware` capability.
    fn discovery_provider_types(&self) -> Vec<ProviderType>;

    /// Validate a package identifier for the given string provider type.
    ///
    /// Returns `Ok(())` for unknown provider types (no constraints apply) and for
    /// provider types that impose no identifier constraints. Returns `Err(message)`
    /// when the identifier violates provider-specific rules.
    fn validate_package_identifier_str(
        &self,
        provider_type: &str,
        value: &str,
    ) -> std::result::Result<(), String>;
}

impl ProviderOps for ProviderRegistry {
    fn validate_config_str(&self, provider_type: &str, config: &serde_json::Value) -> Result<()> {
        ProviderRegistry::validate_config_str(provider_type, config)
    }

    fn mask_config_secrets_str(
        &self,
        provider_type: &str,
        config: &serde_json::Value,
    ) -> serde_json::Value {
        ProviderRegistry::mask_config_secrets_str(provider_type, config)
    }

    fn restore_config_secrets_str(
        &self,
        provider_type: &str,
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
    ) {
        ProviderRegistry::restore_config_secrets_str(provider_type, incoming, existing);
    }

    fn discovery_provider_types(&self) -> Vec<ProviderType> {
        ProviderRegistry::discovery_provider_types()
    }

    fn validate_package_identifier_str(
        &self,
        provider_type: &str,
        value: &str,
    ) -> std::result::Result<(), String> {
        let Ok(pt) = provider_type.parse::<ProviderType>() else {
            return Ok(());
        };
        ProviderRegistry::validate_package_identifier(pt, value)
    }
}
