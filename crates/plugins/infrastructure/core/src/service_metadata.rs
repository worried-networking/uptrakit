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

impl ServiceMetadata {
    pub fn new(
        service_name: String,
        binary_path: Option<PathBuf>,
        version: String,
        deployment_topology: DeploymentTopology,
        reuseport_configured: bool,
        pid_file: Option<PathBuf>,
    ) -> Self {
        Self {
            service_name,
            binary_path,
            version,
            deployment_topology,
            reuseport_configured,
            pid_file,
        }
    }
}

/// Implemented by the controller-standalone; injected into the self-update plugin at construction.
pub trait ServiceMetadataProvider: Send + Sync {
    fn get_metadata(&self) -> ServiceMetadata;
}
