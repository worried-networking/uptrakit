use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::Result;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;

use crate::plugin::PacmanPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexer for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        uptrakit_plugin_infrastructure_core::refresh_package_index_command(
            self.executor.as_ref(),
            CommandSpec::exec("pacman", ["-Sy".to_string()]).privileged(),
            "Pacman package database",
        )
        .await
    }
}
