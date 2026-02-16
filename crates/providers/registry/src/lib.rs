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
