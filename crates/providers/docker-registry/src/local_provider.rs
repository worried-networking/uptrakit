use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_provider_core::command::{run_command, run_command_exec, send_output, shell_escape};
use uptrakit_provider_core::{
    Provider, ProviderError, Result, UpdateContext, UpdateOutputLine, UpdateOutputStream, Version,
};

/// Local provider for Docker Registry.
///
/// Executes updates by pulling the new image tag and optionally running
/// a user-configured restart command.
pub struct DockerRegistryLocalProvider;

impl DockerRegistryLocalProvider {
    /// Create a new Docker Registry local provider.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DockerRegistryLocalProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for DockerRegistryLocalProvider {
    async fn detect_installed_version(&self) -> Result<Option<Version>> {
        Ok(None)
    }

    async fn execute_update(
        &self,
        ctx: &UpdateContext,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        let mut output = String::new();

        let image = &ctx.package_identifier;
        let tag = &ctx.to_version;

        send_output(
            output_tx,
            &format!("Pulling Docker image {image}:{tag}"),
            UpdateOutputStream::Stdout,
        )
        .await;
        output.push_str(&format!("Pulling Docker image {image}:{tag}\n"));

        // Pull the new image using direct exec (no shell) to prevent injection
        // via crafted image names or tag values.
        let image_ref = format!("{image}:{tag}");
        let (cmd_output, _exit_code) =
            run_command_exec("docker", &["pull".to_string(), image_ref], None, output_tx)
                .await
                .map_err(|e| report!(ProviderError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output);

        // Check for restart command in provider config
        if let Some(restart_cmd) = ctx.provider_config.get("restart_command")
            && let Some(cmd_str) = restart_cmd.as_str()
        {
            let cmd = cmd_str
                .replace("{image}", &shell_escape(image))
                .replace("{tag}", &shell_escape(tag))
                .replace("{version}", &shell_escape(&ctx.to_version));

            send_output(
                output_tx,
                &format!("Running restart command: {cmd}"),
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

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = DockerRegistryLocalProvider::new();
        let result = provider.detect_installed_version().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_update_no_restart_command_succeeds_if_docker_available() {
        // This test will fail in environments without docker.
        // We test it conditionally by checking if docker exists.
        let docker_check = tokio::process::Command::new("docker")
            .arg("--version")
            .output()
            .await;
        if docker_check.is_err() {
            // Docker not available, skip test
            return;
        }

        let provider = DockerRegistryLocalProvider::new();
        let (tx, mut rx) = mpsc::channel(100);
        let ctx = UpdateContext {
            to_version: "latest".to_string(),
            package_identifier: "hello-world".to_string(),
            provider_config: serde_json::json!({}),
            release_info: None,
        };
        // This will try to pull the image, which requires network.
        // Just verify the function doesn't panic.
        let _result = provider.execute_update(&ctx, &tx).await;
        rx.close();
        while rx.recv().await.is_some() {}
    }
}
