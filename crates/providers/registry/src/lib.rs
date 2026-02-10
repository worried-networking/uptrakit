//! Provider Registry for Uptrakit
//!
//! This crate provides a centralized registry for provider operations:
//!
//! - **Provider creation**: Create provider instances from configuration
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
//! // Create a provider
//! let provider = ProviderRegistry::create_provider(
//!     ProviderType::GithubReleases,
//!     &config,
//! )?;
//!
//! // Detect installed version
//! let version = provider.detect_installed_version().await?;
//! ```

pub mod error;
pub mod registry;

pub use error::{RegistryError, Result};
pub use registry::ProviderRegistry;

// Re-export commonly used types from provider-core
pub use uptrakit_provider_core::{Provider, ProviderCapability, ProviderType, UpdateContext};
