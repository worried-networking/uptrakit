use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_provider_core::mpsc;
use uptrakit_provider_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_provider_core::{
    Provider, ProviderCapability, ProviderError, ProviderType, ReleaseInfo, Result,
    UpdateOutputLine, UpdateOutputStream, Version,
};

use crate::config::ProxmoxHelperScriptsConfig;

/// Provider for Proxmox Helper Scripts.
///
/// Executes updates by running the helper script via `curl | bash`.
pub struct ProxmoxHelperScriptsProvider {
    config: ProxmoxHelperScriptsConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl ProxmoxHelperScriptsProvider {
    /// Create a new Proxmox Helper Scripts provider with the given configuration.
    pub fn new(config: ProxmoxHelperScriptsConfig, executor: Arc<dyn CommandExecutor>) -> Self {
        Self { config, executor }
    }
}

#[async_trait]
impl Provider for ProxmoxHelperScriptsProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ProxmoxHelperScripts
    }

    fn capabilities(&self) -> &'static [ProviderCapability] {
        &[ProviderCapability::DiscoverLocalSoftware]
    }

    async fn detect_installed_version(&self, _package_identifier: &str) -> Result<Option<Version>> {
        Ok(None)
    }

    async fn execute_update(
        &self,
        _package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        let mut output = String::new();
        let script_url = &self.config.script_url;

        send_output(
            output_tx,
            &format!("Running update script from {script_url}"),
            UpdateOutputStream::Stdout,
        )
        .await;
        output.push_str(&format!("Running update script from {script_url}\n"));

        // Run the helper script via bash, passing the URL as a positional argument
        // (`$1`) to avoid shell interpretation of the URL string.
        let cmd_output = self
            .executor
            .execute(
                &CommandSpec::exec(
                    "bash",
                    [
                        "-c".to_string(),
                        "set -euo pipefail\ncurl -fsSL -- \"$1\" | bash -s -- --update".to_string(),
                        "--".to_string(),
                        script_url.to_string(),
                    ],
                ),
                output_tx,
            )
            .await
            .map_err(|e| report!(ProviderError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output.output);

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_provider_core::LocalCommandExecutor;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn test_config() -> ProxmoxHelperScriptsConfig {
        ProxmoxHelperScriptsConfig {
            script_url: "https://example.com/update.sh".to_string(),
        }
    }

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = ProxmoxHelperScriptsProvider::new(test_config(), test_executor());
        let result = provider.detect_installed_version("example").await.unwrap();
        assert!(result.is_none());
    }
}
