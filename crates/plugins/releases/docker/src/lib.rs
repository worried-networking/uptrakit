pub mod api_types;
pub mod auth;
pub mod config;
#[cfg(feature = "daemon")]
mod credentials;
#[cfg(feature = "daemon")]
mod daemon_client;
mod discovery;
pub mod docker_client;
#[cfg(all(unix, feature = "daemon"))]
mod docker_proxy;
pub mod error;
pub mod image_ref;
pub mod plugin;
pub mod registry;
mod release_fetch;
pub mod surfaces;
mod update;
mod version_detect;

pub use config::DockerConfig;
pub use error::{DockerError, Result};
pub use image_ref::{ImageRef, ParseImageRefError, validate_identifier};
pub use plugin::{DESCRIPTOR, DockerPlugin};
