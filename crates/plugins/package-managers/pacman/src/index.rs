use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::command::CommandSpec;
use uptrakit_plugin_infrastructure_core::{Result, execute_and_capture};

use crate::plugin::PacmanPlugin;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexPlugin for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing Pacman package database");
        execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("pacman", ["-Sy".to_string()]).privileged(),
            "pacman -Sy",
        )
        .await?;

        tracing::info!("Pacman package database refreshed");
        Ok(())
    }
}
