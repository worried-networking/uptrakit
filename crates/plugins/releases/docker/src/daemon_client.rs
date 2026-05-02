use std::sync::Arc;
use std::time::Duration;

use crate::config::ContainerRuntime;
use crate::docker_client::BollardDockerClient;
use crate::docker_client::DockerClient;
use crate::plugin::{DockerPlugin, OpaqueHandle};
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};

impl DockerPlugin {
    /// Replace a stub Docker client with a real [`BollardDockerClient`].
    ///
    /// When the executor supports stdio tunnels (e.g. SSH) and no explicit
    /// `docker_host` is configured, a [`crate::docker_proxy::DockerSocketProxy`]
    /// is started and the client connects to the local proxy socket. Otherwise
    /// falls through to the standard bollard connection logic.
    ///
    /// The `_stub` parameter is the [`crate::docker_client::NoopDockerClient`]
    /// created unconditionally in [`Self::new`]. Accepting it here ensures the
    /// initial binding is read, suppressing `unused_assignments` and `dead_code`
    /// lints, while making it explicit that the daemon path fully replaces the
    /// stub.
    pub(crate) async fn upgrade_to_daemon_client(
        _stub: Arc<dyn DockerClient>,
        _proxy_stub: OpaqueHandle,
        config: &crate::config::DockerConfig,
        executor: &Arc<dyn CommandExecutor>,
    ) -> crate::error::Result<(Arc<dyn DockerClient>, OpaqueHandle)> {
        // When the executor supports stdio tunnels and no explicit docker_host
        // is configured, start a local Unix socket proxy. This avoids bollard's
        // SSH codepath (which spawns a second SSH connection via the system ssh
        // binary) and instead tunnels Docker API traffic over the existing
        // russh session.
        #[cfg(unix)]
        if executor.supports_stdio_tunnel() && config.docker_host.is_none() {
            let dial_cmd = match config.container_runtime {
                ContainerRuntime::Docker => "docker system dial-stdio",
                ContainerRuntime::Podman => "podman system dial-stdio",
                ContainerRuntime::Auto => "docker system dial-stdio", // overridden after detection
            };
            let proxy =
                crate::docker_proxy::DockerSocketProxy::start(Arc::clone(executor), dial_cmd)
                    .await?;
            let uri = proxy.socket_uri();
            tracing::info!(
                proxy_socket = %uri,
                "Docker socket proxy started; connecting bollard via proxy"
            );
            let client = Arc::new(BollardDockerClient::new(Some(&uri), None, None)?);
            let handle: Box<dyn std::any::Any + Send + Sync> = Box::new(proxy);
            return Ok((client, Some(handle)));
        }

        let client = Arc::new(BollardDockerClient::new(
            config.docker_host.as_deref(),
            config.ssh_key_path.as_deref(),
            config.tls.as_ref(),
        )?);
        Ok((client, None))
    }

    /// Returns the `dial-stdio` command string for the configured/detected runtime.
    ///
    /// Explicit `Docker`/`Podman` config always wins. In `Auto` mode the
    /// previously detected runtime is used (defaulting to `docker` if
    /// detection has not yet run).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only called in tests; retained for integration verification"
        )
    )]
    pub(crate) fn effective_dial_stdio_command(&self) -> &'static str {
        match self.config.container_runtime {
            ContainerRuntime::Docker => "docker system dial-stdio",
            ContainerRuntime::Podman => "podman system dial-stdio",
            ContainerRuntime::Auto => match *self.detected_runtime.lock() {
                Some(ContainerRuntime::Podman) => "podman system dial-stdio",
                _ => "docker system dial-stdio",
            },
        }
    }

    /// Returns the effective registry authentication for the given image reference.
    ///
    /// Resolution order:
    /// 1. Explicit `config.auth` (always wins).
    /// 2. System credentials from `~/.docker/config.json` when `use_system_credentials` is true.
    /// 3. `None` (unauthenticated).
    pub(crate) async fn effective_auth(&self, image: &str) -> Option<crate::config::DockerAuth> {
        // Explicit auth always wins.
        if self.config.auth.is_some() {
            return self.config.auth.clone();
        }

        if !self.config.use_system_credentials {
            return None;
        }

        // Determine whether we're accessing a remote host.
        let executor = match self.require_executor() {
            Ok(e) => e,
            Err(_) => return None,
        };
        let is_remote = executor.supports_stdio_tunnel();

        // Parse registry from the image reference.
        let registry = image
            .parse::<crate::image_ref::ImageRef>()
            .map(|r| r.server_address())
            .unwrap_or_else(|_| image.to_string());

        crate::credentials::resolve_system_credentials(
            &registry,
            executor,
            is_remote,
            &self.credential_cache,
        )
        .await
    }

    /// Probe the executor for the available container runtime and, when running
    /// over an SSH stdio tunnel, restart the proxy with the detected command.
    ///
    /// Returns `Some(runtime)` when a runtime is found, `None` when neither
    /// Docker nor Podman is available, or an `Err` on unexpected failure.
    pub(crate) async fn detect_and_apply_runtime(
        &self,
    ) -> crate::error::Result<Option<ContainerRuntime>> {
        const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

        let executor = self.require_executor().map_err(|e| rootcause::report!(e))?;

        // Helper: run a shell command via the executor and return true if exit 0.
        let probe = |cmd: &'static str| {
            let executor = Arc::clone(executor);
            async move {
                tokio::time::timeout(
                    PROBE_TIMEOUT,
                    executor.execute_quiet(&CommandSpec::shell(cmd)),
                )
                .await
                .ok()
                .and_then(|r| r.ok())
                .map(|o| o.exit_code == 0)
                .unwrap_or(false)
            }
        };

        let runtime = if probe("command -v docker >/dev/null 2>&1").await {
            tracing::debug!("detected Docker runtime");
            ContainerRuntime::Docker
        } else if probe("command -v podman >/dev/null 2>&1").await {
            tracing::debug!("detected Podman runtime");
            ContainerRuntime::Podman
        } else {
            return Ok(None);
        };

        // When the executor supports stdio tunnels and no explicit docker_host
        // is configured, restart the proxy with the detected runtime's command
        // so all subsequent bollard calls use the correct binary.
        #[cfg(unix)]
        if executor.supports_stdio_tunnel() && self.config.docker_host.is_none() {
            let dial_cmd = match runtime {
                ContainerRuntime::Docker => "docker system dial-stdio",
                ContainerRuntime::Podman => "podman system dial-stdio",
                ContainerRuntime::Auto => "docker system dial-stdio",
            };

            tracing::info!(
                runtime = ?runtime,
                cmd = %dial_cmd,
                "restarting Docker socket proxy with detected runtime"
            );

            let proxy =
                crate::docker_proxy::DockerSocketProxy::start(Arc::clone(executor), dial_cmd)
                    .await
                    .map_err(|e| {
                        rootcause::report!(crate::error::DockerError::DaemonConnection(
                            e.to_string()
                        ))
                    })?;
            let uri = proxy.socket_uri();
            let new_client = Arc::new(BollardDockerClient::new(Some(&uri), None, None)?)
                as Arc<dyn DockerClient>;
            let handle: Box<dyn std::any::Any + Send + Sync> = Box::new(proxy);

            *self.docker_client.lock() = new_client;
            *self.proxy_handle.lock() = Some(handle);
        }

        Ok(Some(runtime))
    }
}
