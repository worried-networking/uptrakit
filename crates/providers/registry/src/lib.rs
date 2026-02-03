//! Provider Registry for Uptrakit
//!
//! This crate provides a centralized registry for provider operations:
//!
//! - **Provider creation**: Create local and remote provider instances from configuration
//! - **Configuration validation**: Validate provider-specific configuration JSON
//! - **Secret management**: Mask and restore sensitive fields in configuration
//!
//! # Example
//!
//! ```ignore
//! use uptrakit_provider_core::ProviderType;
//! use uptrakit_provider_registry::ProviderRegistry;
//!
//! // Validate configuration
//! let config = serde_json::json!({
//!     "owner": "octocat",
//!     "repo": "hello-world"
//! });
//! ProviderRegistry::validate_config(ProviderType::GithubReleases, &config)?;
//!
//! // Create a local provider
//! let provider = ProviderRegistry::create_local_provider(
//!     ProviderType::GithubReleases,
//!     "octocat/hello-world",
//!     &config,
//! )?;
//!
//! // Detect installed version
//! let version = provider.detect_installed_version().await?;
//! ```

pub mod error;
pub mod registry;
pub mod secrets;

pub use error::{RegistryError, Result};
pub use registry::ProviderRegistry;

// Re-export commonly used types from provider-core
pub use uptrakit_provider_core::{LocalProvider, ProviderType, RemoteProvider};
