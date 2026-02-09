use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_provider_core::command::{run_command_exec, send_output};
use uptrakit_provider_core::{
    Provider, ProviderCapability, ProviderError, Result, UpdateContext, UpdateOutputLine,
    UpdateOutputStream, Version,
};

/// Local provider for Proxmox Helper Scripts.
///
/// Executes updates by running the helper script via `curl | bash`.
pub struct ProxmoxHelperScriptsLocalProvider {
    /// Package identifier (script name).
    pub package_identifier: String,
}

impl ProxmoxHelperScriptsLocalProvider {
    /// Create a new Proxmox Helper Scripts local provider.
    pub fn new(package_identifier: String) -> Self {
        Self { package_identifier }
    }
}

#[async_trait]
impl Provider for ProxmoxHelperScriptsLocalProvider {
    fn capabilities(&self) -> &'static [ProviderCapability] {
        &[ProviderCapability::DiscoverLocalSoftware]
    }

    async fn detect_installed_version(&self) -> Result<Option<Version>> {
        Ok(None)
    }

    async fn execute_update(
        &self,
        ctx: &UpdateContext,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        let mut output = String::new();

        let script_url = ctx
            .provider_config
            .get("script_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| report!(ProviderError::MissingConfig("script_url".to_string())))?;

        send_output(
            output_tx,
            &format!("Running update script from {script_url}"),
            UpdateOutputStream::Stdout,
        )
        .await;
        output.push_str(&format!("Running update script from {script_url}\n"));

        // Run the helper script via bash, passing the URL as a positional argument
        // (`$1`) to avoid shell interpretation of the URL string.
        let (cmd_output, _exit_code) = run_command_exec(
            "bash",
            &[
                "-c".to_string(),
                "set -euo pipefail\ncurl -fsSL -- \"$1\" | bash -s -- --update".to_string(),
                "--".to_string(),
                script_url.to_string(),
            ],
            None,
            output_tx,
        )
        .await
        .map_err(|e| report!(ProviderError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output);

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = ProxmoxHelperScriptsLocalProvider::new("test-script".to_string());
        let result = provider.detect_installed_version().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_update_missing_script_url_returns_error() {
        let provider = ProxmoxHelperScriptsLocalProvider::new("test-script".to_string());
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateContext {
            to_version: "1.0.0".to_string(),
            package_identifier: "test-script".to_string(),
            provider_config: serde_json::json!({}),
            release_info: None,
        };
        let result = provider.execute_update(&ctx, &tx).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err.current_context(),
                ProviderError::MissingConfig(field) if field == "script_url"
            ),
            "Expected MissingConfig(script_url), got: {err}"
        );
    }
}
