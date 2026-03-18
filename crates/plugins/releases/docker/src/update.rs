use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;

use crate::image_ref::ImageRef;
use crate::plugin::DockerPlugin;
use uptrakit_plugin_infrastructure_core::command::{CommandSpec, send_output, shell_escape};
use uptrakit_plugin_infrastructure_core::{
    OutputStreamType, PluginError, ReleaseInfo, UpdateOutputLine, mpsc,
};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for DockerPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_plugin_infrastructure_core::Result<String> {
        let executor = self.executor.as_ref().ok_or_else(|| {
            report!(PluginError::Configuration(
                "execute_update requires a POSIX executor (not available on controller)"
                    .to_string()
            ))
        })?;

        let ir: ImageRef =
            package_identifier
                .parse()
                .map_err(|e: crate::image_ref::ParseImageRefError| {
                    PluginError::PluginInternal(e.to_string())
                })?;

        let image = &ir.image;
        // Always pull by the configured tag (e.g. "latest"), not by digest.
        let tag = self.config.resolved_tracked_tag(&ir.tag);
        let full_ref = format!("{image}:{tag}");
        let mut output = String::new();

        // Pre-pull: collect running/stopped state of the containers to recreate.
        //
        // When the package identifier carries a `#container_name` qualifier (e.g.
        // `nginx:latest#web-server`), only that specific container is targeted.
        // Without a qualifier all containers using this image are managed, which
        // preserves behaviour for items created before per-container tracking was
        // introduced.
        let client = Arc::clone(&*self.docker_client.lock());
        let all_containers = client
            .list_containers_for_image(&full_ref)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    image = %full_ref,
                    error = %e,
                    "failed to list containers before pull; recreation will be skipped"
                );
                vec![]
            });

        let containers_before: Vec<_> = if let Some(ref target) = ir.container_name {
            all_containers
                .into_iter()
                .filter(|c| c.name == *target && self.container_passes_label_filter(&c.labels))
                .collect()
        } else {
            all_containers
                .into_iter()
                .filter(|c| self.container_passes_label_filter(&c.labels))
                .collect()
        };

        send_output(
            output_tx,
            &format!("Pulling Docker image {image}:{tag}"),
            OutputStreamType::Stdout,
        )
        .await;
        output.push_str(&format!("Pulling Docker image {image}:{tag}\n"));

        tracing::debug!(image = %image, tag = %tag, "pulling Docker image");
        // Use daemon-sourced credentials when the daemon feature is enabled
        // (queries the Docker credential store at runtime); otherwise fall back
        // to the static auth configured in the plugin config.
        #[allow(unused_variables)]
        let auth: Option<crate::config::DockerAuth> = self.config.auth.clone();
        #[cfg(feature = "daemon")]
        let auth = self.effective_auth(image).await;
        let client = Arc::clone(&*self.docker_client.lock());
        let pull_output = client
            .pull_image(image, tag, auth.as_ref(), output_tx)
            .await
            .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
        tracing::debug!("Docker image pull completed");
        output.push_str(&pull_output);

        // Run compose_restart if configured.
        // Direction: any containers running before pull -> `up -d` (recreate and start);
        // all stopped -> `up --no-start` (recreate without starting).
        if let Some(ref cr) = self.config.compose_restart {
            let any_running = containers_before.iter().any(|c| c.is_running);

            let mut parts: Vec<String> = Vec::new();

            if let Some(ref working_dir) = cr.working_dir {
                parts.push(format!("cd {}", shell_escape(working_dir)));
                parts.push("&&".to_string());
            }

            parts.push("docker".to_string());
            parts.push("compose".to_string());

            if let Some(ref file) = cr.compose_file {
                parts.push("-f".to_string());
                parts.push(shell_escape(file));
            }

            parts.push("up".to_string());
            if any_running {
                parts.push("-d".to_string());
            } else {
                parts.push("--no-start".to_string());
            }

            if let Some(ref service) = cr.service {
                parts.push(shell_escape(service));
            }

            let cmd = parts.join(" ");
            tracing::debug!(command = %cmd, "running docker compose restart");
            let compose_msg = format!("Running docker compose: {cmd}");
            send_output(output_tx, &compose_msg, OutputStreamType::Stdout).await;
            output.push_str(&compose_msg);
            output.push('\n');

            let cmd_output = executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
                .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            output.push_str(&cmd_output.output);
        }

        // Run post_pull_command if configured.
        if let Some(ref cmd_str) = self.config.post_pull_command {
            // Try to get local digest for {digest} substitution.
            let client = Arc::clone(&*self.docker_client.lock());
            let digest = match client.inspect_image(&full_ref).await {
                Ok(Some(d)) => d.digest,
                _ => String::new(),
            };

            let cmd = cmd_str
                .replace("{image}", &shell_escape(image))
                .replace("{tag}", &shell_escape(tag))
                .replace("{digest}", &shell_escape(&digest));

            tracing::debug!(command = %cmd, "running post-pull command");
            send_output(
                output_tx,
                &format!("Running post-pull command: {cmd}"),
                OutputStreamType::Stdout,
            )
            .await;

            let cmd_output = executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
                .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            output.push_str(&cmd_output.output);
        }

        // Auto-recreate containers when neither compose_restart nor post_pull_command
        // is configured.  Containers are recreated in-place, preserving all settings.
        // Running containers are started again; stopped containers remain stopped.
        if self.config.compose_restart.is_none() && self.config.post_pull_command.is_none() {
            for container in &containers_before {
                tracing::info!(
                    container = %container.name,
                    was_running = container.is_running,
                    "recreating container after image update"
                );
                let line = format!(
                    "Recreating container {} (was {})",
                    container.name,
                    if container.is_running {
                        "running"
                    } else {
                        "stopped"
                    }
                );
                send_output(output_tx, &line, OutputStreamType::Stdout).await;
                output.push_str(&line);
                output.push('\n');

                let client = Arc::clone(&*self.docker_client.lock());
                client
                    .recreate_container(&container.name, container.is_running)
                    .await
                    .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            }
        }

        Ok(output)
    }
}
