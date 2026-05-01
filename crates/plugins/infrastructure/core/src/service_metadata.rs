// crates/plugins/infrastructure/core/src/service_metadata.rs
use std::path::PathBuf;

/// Metadata about a running uptrakit service, provided by the controller
/// to the embedded self-update discovery plugin.
#[non_exhaustive]
pub struct ServiceMetadata {
    pub service_name: String,
    pub binary_path: Option<PathBuf>,
    pub version: String,
    pub deployment_topology: DeploymentTopology,
    pub reuseport_configured: bool,
    pub pid_file: Option<PathBuf>,
}

/// The deployment topology of the running service.
#[non_exhaustive]
pub enum DeploymentTopology {
    /// Unix only (Linux + macOS). Windows deferred.
    UnixBinary,
    DockerContainer {
        image: String,
        container_name: String,
    },
}

/// Implemented by the controller-standalone; injected into the self-update plugin at construction.
pub trait ServiceMetadataProvider: Send + Sync {
    fn get_metadata(&self) -> ServiceMetadata;
}
