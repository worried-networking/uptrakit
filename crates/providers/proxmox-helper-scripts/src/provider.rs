use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_provider_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_provider_core::mpsc;
use uptrakit_provider_core::{
    DiscoveredSoftware, Provider, ProviderCapability, ProviderError, ProviderType, ReleaseInfo,
    Result, UpdateOutputLine, UpdateOutputStream, Version,
};

use crate::config::ProxmoxHelperScriptsConfig;
use crate::discovery::{
    UPDATE_SCRIPT_PATH, parse_phs_scripts, parse_version_file, slug_to_display_name,
    validate_package_identifier,
};

/// Provider for Proxmox Helper Scripts.
///
/// Discovers PHS-managed software by parsing `/usr/bin/update` for
/// community-scripts references, detects installed versions from
/// `$HOME/.{slug}` files, and executes updates via `curl | bash`.
pub struct ProxmoxHelperScriptsProvider {
    config: ProxmoxHelperScriptsConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl ProxmoxHelperScriptsProvider {
    /// Create a new Proxmox Helper Scripts provider with the given configuration.
    pub fn new(config: ProxmoxHelperScriptsConfig, executor: Arc<dyn CommandExecutor>) -> Self {
        Self { config, executor }
    }

    /// Read the user's HOME directory via `printenv HOME`.
    async fn resolve_home(&self) -> Result<String> {
        let output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "printenv",
                ["HOME".to_string()],
            ))
            .await
            .map_err(|e| {
                report!(ProviderError::ProviderInternal(format!(
                    "failed to resolve HOME: {e}"
                )))
            })?;

        let home = output.output.trim().to_string();
        if home.is_empty() {
            bail!(ProviderError::ProviderInternal(
                "HOME environment variable is empty".to_string()
            ));
        }
        Ok(home)
    }

    /// Try to read a version file at the given path. Returns `None` if the
    /// file does not exist or cannot be read.
    async fn try_read_version_file(&self, path: &str) -> Option<String> {
        let output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "cat",
                ["--".to_string(), path.to_string()],
            ))
            .await
            .ok()?;

        parse_version_file(&output.output).map(String::from)
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

    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::debug!("reading PHS update script from {UPDATE_SCRIPT_PATH}");

        let update_content = match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "cat",
                ["--".to_string(), UPDATE_SCRIPT_PATH.to_string()],
            ))
            .await
        {
            Ok(output) => output.output,
            Err(_) => {
                tracing::debug!("no PHS update script found at {UPDATE_SCRIPT_PATH}");
                return Ok(vec![]);
            }
        };

        let scripts = parse_phs_scripts(&update_content);
        if scripts.is_empty() {
            tracing::debug!("no PHS script references found in {UPDATE_SCRIPT_PATH}");
            return Ok(vec![]);
        }

        let home = self.resolve_home().await?;
        let mut discovered = Vec::with_capacity(scripts.len());

        for script in &scripts {
            let version_path = format!("{home}/.{}", script.slug);
            let installed_version = self
                .try_read_version_file(&version_path)
                .await
                .map(Version::new);

            tracing::debug!(
                slug = %script.slug,
                version = ?installed_version,
                "discovered PHS software"
            );

            discovered.push(DiscoveredSoftware {
                package_identifier: script.slug.clone(),
                name: slug_to_display_name(&script.slug),
                installed_version,
                extra: Some(serde_json::json!({
                    "script_url": script.script_url,
                })),
            });
        }

        Ok(discovered)
    }

    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        validate_package_identifier(package_identifier)?;

        let home = self.resolve_home().await?;
        let version_path = format!("{home}/.{package_identifier}");

        tracing::debug!(
            path = %version_path,
            "reading PHS version file"
        );

        Ok(self
            .try_read_version_file(&version_path)
            .await
            .map(Version::new))
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

    #[test]
    fn capabilities_include_discover_local_software() {
        let provider = ProxmoxHelperScriptsProvider::new(test_config(), test_executor());
        assert!(provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
        assert_eq!(provider.capabilities().len(), 1);
    }

    #[test]
    fn provider_type_is_proxmox_helper_scripts() {
        let provider = ProxmoxHelperScriptsProvider::new(test_config(), test_executor());
        assert_eq!(provider.provider_type(), ProviderType::ProxmoxHelperScripts);
    }

    #[tokio::test]
    async fn detect_installed_version_rejects_invalid_identifier() {
        let provider = ProxmoxHelperScriptsProvider::new(test_config(), test_executor());
        let result = provider.detect_installed_version("").await;
        assert!(result.is_err());

        let result = provider.detect_installed_version("../etc/passwd").await;
        assert!(result.is_err());

        let result = provider.detect_installed_version("UPPER").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn detect_installed_version_returns_none_for_missing_file() {
        let provider = ProxmoxHelperScriptsProvider::new(test_config(), test_executor());
        // A slug that almost certainly has no version file
        let result = provider
            .detect_installed_version("nonexistent-phs-app-test")
            .await;
        assert!(result.is_ok());
        assert!(result.expect("should succeed").is_none());
    }

    #[tokio::test]
    async fn discover_software_returns_empty_without_update_script() {
        // On a non-PHS system, /usr/bin/update likely doesn't exist
        let provider = ProxmoxHelperScriptsProvider::new(test_config(), test_executor());
        let result = provider.discover_software().await;
        assert!(result.is_ok());
        // It either returns empty (no update script) or parses whatever is there
        // In either case, it should not error
    }
}
