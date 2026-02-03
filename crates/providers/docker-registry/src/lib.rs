pub mod api_types;
pub mod auth;
pub mod config;
pub mod error;
pub mod local_provider;
pub mod provider;
pub mod registry;
pub mod tag;

pub use config::DockerRegistryConfig;
pub use error::{DockerRegistryError, Result};
pub use local_provider::DockerRegistryLocalProvider;
pub use provider::DockerRegistryProvider;
