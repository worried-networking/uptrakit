// crates/core/controller-runtime/src/embedded/metadata_runtime.rs
use uptrakit_plugin_infrastructure_core::service_metadata::{
    DeploymentTopology, ServiceMetadata, ServiceMetadataProvider,
};

/// Implements [`ServiceMetadataProvider`] by reading the running binary path from
/// [`std::env::current_exe()`].
pub(crate) struct ControllerMetadataProvider {
    service_name: String,
    version: String,
    reuseport_configured: bool,
    pid_file: Option<std::path::PathBuf>,
}

impl ControllerMetadataProvider {
    pub(crate) fn new(
        service_name: String,
        version: String,
        reuseport_configured: bool,
        pid_file: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            service_name,
            version,
            reuseport_configured,
            pid_file,
        }
    }
}

impl ServiceMetadataProvider for ControllerMetadataProvider {
    fn get_metadata(&self) -> ServiceMetadata {
        let binary_path = std::env::current_exe().ok();
        ServiceMetadata::new(
            self.service_name.clone(),
            binary_path,
            self.version.clone(),
            DeploymentTopology::UnixBinary,
            self.reuseport_configured,
            self.pid_file.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_command::NoopCommandExecutor;
    use uptrakit_plugin_infrastructure_core::{HostRuntime, MetadataAwareHostRuntime};

    #[test]
    fn test_metadata_aware_host_runtime_returns_some_provider() {
        let inner = uptrakit_plugin_infrastructure_core::construct_host_runtime(
            Arc::new(NoopCommandExecutor),
            Default::default(),
        );
        // Build a ControllerMetadataProvider
        let provider = ControllerMetadataProvider::new(
            "uptrakit-controller".to_string(),
            "1.0.0".to_string(),
            false,
            None,
        );
        let runtime = MetadataAwareHostRuntime::new(inner, Arc::new(provider));
        assert!(runtime.metadata_provider().is_some());
    }
}
