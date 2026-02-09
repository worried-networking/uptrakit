use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_provider_core::command::{run_command, send_output, shell_escape};
use uptrakit_provider_core::{
    Provider, ProviderError, Result, UpdateContext, UpdateOutputLine, UpdateOutputStream, Version,
};

use crate::config::GitHubConfig;

/// Local provider for GitHub Releases.
///
/// Executes updates by running the user-configured `install_command` with
/// shell-escaped variable substitutions.
pub struct GitHubLocalProvider {
    /// Provider configuration.
    pub config: GitHubConfig,
    /// Package identifier (owner/repo).
    pub package_identifier: String,
}

impl GitHubLocalProvider {
    /// Create a new GitHub local provider.
    pub fn new(config: GitHubConfig, package_identifier: String) -> Self {
        Self {
            config,
            package_identifier,
        }
    }
}

#[async_trait]
impl Provider for GitHubLocalProvider {
    async fn detect_installed_version(&self) -> Result<Option<Version>> {
        Ok(None)
    }

    async fn execute_update(
        &self,
        ctx: &UpdateContext,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        let mut output = String::new();

        let release_info = ctx
            .release_info
            .as_ref()
            .ok_or_else(|| report!(ProviderError::MissingReleaseInfo))?;

        send_output(
            output_tx,
            &format!(
                "Downloading release {} from {}",
                release_info.tag, release_info.release_url
            ),
            UpdateOutputStream::Stdout,
        )
        .await;
        output.push_str(&format!(
            "Downloading release {} from {}\n",
            release_info.tag, release_info.release_url
        ));

        if let Some(install_cmd) = ctx.provider_config.get("install_command") {
            if let Some(cmd_str) = install_cmd.as_str() {
                let cmd = cmd_str
                    .replace("{version}", &shell_escape(&ctx.to_version))
                    .replace("{tag}", &shell_escape(&release_info.tag))
                    .replace(
                        "{package_identifier}",
                        &shell_escape(&ctx.package_identifier),
                    );

                send_output(
                    output_tx,
                    &format!("Running install command: {cmd}"),
                    UpdateOutputStream::Stdout,
                )
                .await;

                match run_command(&cmd, output_tx).await {
                    Ok(cmd_output) => {
                        output.push_str(&cmd_output);
                    }
                    Err(e) => {
                        return Err(report!(ProviderError::InstallFailed(e.to_string())));
                    }
                }
            }
        } else {
            send_output(
                output_tx,
                "No install_command configured, skipping automated installation",
                UpdateOutputStream::Stdout,
            )
            .await;
            output.push_str("No install_command configured, skipping automated installation\n");
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> GitHubConfig {
        GitHubConfig {
            owner: "octocat".to_string(),
            repo: "hello-world".to_string(),
            auth_token: None,
            api_base_url: None,
            include_prereleases: false,
            tag_strip_prefix: "v".to_string(),
            asset_patterns: vec![],
        }
    }

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = GitHubLocalProvider::new(test_config(), "octocat/hello-world".to_string());
        let result = provider.detect_installed_version().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_update_missing_release_info_returns_error() {
        let provider = GitHubLocalProvider::new(test_config(), "octocat/hello-world".to_string());
        let (tx, _rx) = mpsc::channel(100);
        let ctx = UpdateContext {
            to_version: "1.0.0".to_string(),
            package_identifier: "octocat/hello-world".to_string(),
            provider_config: serde_json::json!({}),
            release_info: None,
        };
        let result = provider.execute_update(&ctx, &tx).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), ProviderError::MissingReleaseInfo),
            "Expected MissingReleaseInfo, got: {err}"
        );
    }

    #[tokio::test]
    async fn execute_update_no_install_command_succeeds() {
        let provider = GitHubLocalProvider::new(test_config(), "octocat/hello-world".to_string());
        let (tx, mut rx) = mpsc::channel(100);
        let ctx = UpdateContext {
            to_version: "1.0.0".to_string(),
            package_identifier: "octocat/hello-world".to_string(),
            provider_config: serde_json::json!({}),
            release_info: Some(uptrakit_provider_core::ReleaseInfo {
                tag: "v1.0.0".to_string(),
                release_url: "https://example.com".to_string(),
                assets: vec![],
            }),
        };
        let result = provider.execute_update(&ctx, &tx).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("No install_command configured"));
        rx.close();
        while rx.recv().await.is_some() {}
    }
}
