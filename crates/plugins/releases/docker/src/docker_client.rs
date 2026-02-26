//! Docker daemon abstraction.
//!
//! Defines the [`DockerClient`] trait for injecting different implementations
//! (real bollard client in production, mock in tests), and the production
//! [`BollardDockerClient`] that communicates with the Docker daemon.

use async_trait::async_trait;
use futures_util::StreamExt;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::send_output;
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{OutputStreamType, UpdateOutputLine};

use crate::config::DockerAuth;
use crate::error::{DockerError, Result};

// ── Public result types ───────────────────────────────────────────────────────

/// Local digest information for an image.
#[derive(Debug, Clone)]
pub struct LocalImageDigest {
    /// The SHA-256 digest string (e.g. `"sha256:abc…"`).
    pub digest: String,
}

/// Information about a running or stopped container.
#[derive(Debug, Clone)]
pub struct LocalContainerInfo {
    /// The image reference (e.g. `"nginx:latest"` or a bare SHA).
    pub image: String,
    /// Container names (leading `'/'` stripped).
    pub names: Vec<String>,
}

// ── Trait ────────────────────────────────────────────────────────────────────

/// Docker daemon operations needed by the Docker plugin.
///
/// The trait exists so that [`crate::plugin::DockerPlugin`] can be
/// tested with a mock implementation that does not require a live Docker daemon.
#[async_trait]
pub(crate) trait DockerClient: Send + Sync {
    /// Pull `image:tag` from the registry, streaming progress through `output_tx`.
    ///
    /// Returns the accumulated progress output as a `String`.
    async fn pull_image(
        &self,
        image: &str,
        tag: &str,
        auth: Option<&DockerAuth>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String>;

    /// Inspect a local image by full reference (`image:tag`).
    ///
    /// Returns `None` if the image is not present locally or has no
    /// `RepoDigests` (locally built images without registry provenance).
    async fn inspect_image(&self, full_ref: &str) -> Result<Option<LocalImageDigest>>;

    /// List running (and optionally stopped) containers.
    ///
    /// When `all` is `true`, stopped containers are included.
    async fn list_containers(&self, all: bool) -> Result<Vec<LocalContainerInfo>>;
}

// ── Production implementation ────────────────────────────────────────────────

/// Docker client backed by the bollard library.
///
/// Communicates with the Docker daemon directly over its Unix socket (or
/// Windows named pipe / TCP / SSH), with no dependency on the `docker` CLI.
pub(crate) struct BollardDockerClient {
    docker: bollard::Docker,
}

impl BollardDockerClient {
    /// Create a new client connected to the Docker daemon.
    ///
    /// - `None` → use [`bollard::Docker::connect_with_defaults`] (reads
    ///   `DOCKER_HOST`, falls back to the platform default socket).
    /// - `Some("unix:///path")` → connect to that Unix socket.
    /// - `Some("tcp://host:port")` → plain HTTP connection.
    /// - `Some("ssh://user@host[:port]")` → SSH tunnel (**requires the `ssh`
    ///   Cargo feature**).
    pub(crate) fn new(docker_host: Option<&str>, ssh_key_path: Option<&str>) -> Result<Self> {
        let docker = Self::connect(docker_host, ssh_key_path)?;
        Ok(Self { docker })
    }

    fn connect(docker_host: Option<&str>, ssh_key_path: Option<&str>) -> Result<bollard::Docker> {
        use bollard::API_DEFAULT_VERSION;
        const TIMEOUT: u64 = 120;

        match docker_host {
            None => bollard::Docker::connect_with_defaults().context_to::<DockerError>(),

            Some(h) if h.starts_with("unix://") => {
                let path = &h["unix://".len()..];
                bollard::Docker::connect_with_socket(path, TIMEOUT, API_DEFAULT_VERSION)
                    .context_to::<DockerError>()
            }

            Some(h) if h.starts_with("ssh://") => {
                #[cfg(feature = "ssh")]
                {
                    // Use bollard's first-class SSH connector (openssh crate, system ssh
                    // binary with docker system dial-stdio).  Pass the key path directly
                    // so the stored per-host key is used rather than relying on default
                    // ~/.ssh/ locations.  When ssh_key_path is None bollard falls back to
                    // SSH agent or default key files.
                    bollard::Docker::connect_with_ssh(
                        h,
                        TIMEOUT,
                        API_DEFAULT_VERSION,
                        ssh_key_path.map(str::to_string),
                    )
                    .context_to::<DockerError>()
                }
                #[cfg(not(feature = "ssh"))]
                {
                    let _ = (h, ssh_key_path);
                    bail!(DockerError::Configuration(
                        "SSH Docker connections require the 'ssh' Cargo feature to be enabled"
                            .to_string()
                    ))
                }
            }

            Some(h) => bollard::Docker::connect_with_http(h, TIMEOUT, API_DEFAULT_VERSION)
                .context_to::<DockerError>(),
        }
    }
}

#[async_trait]
impl DockerClient for BollardDockerClient {
    async fn pull_image(
        &self,
        image: &str,
        tag: &str,
        auth: Option<&DockerAuth>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        tracing::debug!(image = %image, "starting Docker image pull");
        let credentials = auth.map(|a| map_auth_to_credentials(image, a));

        let mut stream = self.docker.create_image(
            Some(bollard::query_parameters::CreateImageOptions {
                from_image: Some(image.to_string()),
                tag: Some(tag.to_string()),
                ..Default::default()
            }),
            None,
            credentials,
        );

        let mut output = String::new();

        while let Some(item) = stream.next().await {
            let info = item.context_to::<DockerError>()?;

            if let Some(ref detail) = info.error_detail {
                let msg = detail.message.as_deref().unwrap_or("docker pull error");
                tracing::warn!(error = %msg, "Docker pull stream error");
                bail!(DockerError::PullFailed(msg.to_string()));
            }

            let line = format_progress_line(info.id.as_deref(), info.status.as_deref(), None);
            if !line.is_empty() {
                send_output(output_tx, &line, OutputStreamType::Stdout).await;
                output.push_str(&line);
                output.push('\n');
            }
        }

        tracing::debug!("Docker image pull stream completed");
        Ok(output)
    }

    async fn inspect_image(&self, full_ref: &str) -> Result<Option<LocalImageDigest>> {
        match self.docker.inspect_image(full_ref).await {
            Ok(info) => {
                let digest = info
                    .repo_digests
                    .as_deref()
                    .and_then(|ds| ds.first())
                    .and_then(|d| d.split('@').nth(1))
                    .map(|d| d.to_string());

                Ok(digest.map(|d| LocalImageDigest { digest: d }))
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(e) => Err(report!(DockerError::DaemonConnection(e.to_string()))),
        }
    }

    async fn list_containers(&self, all: bool) -> Result<Vec<LocalContainerInfo>> {
        use bollard::query_parameters::ListContainersOptions;

        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all,
                ..Default::default()
            }))
            .await
            .context_to::<DockerError>()?;

        let infos = containers
            .into_iter()
            .map(|c| LocalContainerInfo {
                image: c.image.unwrap_or_default(),
                names: c
                    .names
                    .unwrap_or_default()
                    .into_iter()
                    .map(|n| n.trim_start_matches('/').to_string())
                    .collect(),
            })
            .collect();

        Ok(infos)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Map a [`DockerAuth`] config value to bollard credentials.
fn map_auth_to_credentials(image: &str, auth: &DockerAuth) -> bollard::auth::DockerCredentials {
    use crate::image_ref::ImageRef;
    let server_address = image.parse::<ImageRef>().map(|r| r.server_address()).ok();

    match auth {
        DockerAuth::Basic { username, password } => bollard::auth::DockerCredentials {
            username: Some(username.clone()),
            password: Some(password.expose_secret().to_string()),
            serveraddress: server_address,
            ..Default::default()
        },
        DockerAuth::Bearer { token } => bollard::auth::DockerCredentials {
            registrytoken: Some(token.expose_secret().to_string()),
            ..Default::default()
        },
    }
}

/// Format a single pull-progress event as `"{id}: {status} {progress}"`,
/// omitting any empty components.
fn format_progress_line(id: Option<&str>, status: Option<&str>, progress: Option<&str>) -> String {
    let status = status.unwrap_or("").trim();
    let progress = progress.unwrap_or("").trim();

    let mut status_progress = String::new();
    if !status.is_empty() {
        status_progress.push_str(status);
    }
    if !progress.is_empty() {
        if !status_progress.is_empty() {
            status_progress.push(' ');
        }
        status_progress.push_str(progress);
    }

    if status_progress.is_empty() {
        return String::new();
    }

    match id {
        Some(id) if !id.trim().is_empty() => format!("{}: {status_progress}", id.trim()),
        _ => status_progress,
    }
}

// ── Test mock ────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) struct MockDockerClient {
    pub pull_output: String,
    pub pull_should_fail: bool,
    pub inspect_result: Option<String>, // Some(digest) or None
    pub containers: Vec<LocalContainerInfo>,
}

#[cfg(test)]
#[async_trait]
impl DockerClient for MockDockerClient {
    async fn pull_image(
        &self,
        _image: &str,
        _tag: &str,
        _auth: Option<&DockerAuth>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        if self.pull_should_fail {
            bail!(DockerError::PullFailed("mock pull failure".to_string()));
        }
        send_output(output_tx, &self.pull_output, OutputStreamType::Stdout).await;
        Ok(self.pull_output.clone())
    }

    async fn inspect_image(&self, _full_ref: &str) -> Result<Option<LocalImageDigest>> {
        Ok(self
            .inspect_result
            .clone()
            .map(|d| LocalImageDigest { digest: d }))
    }

    async fn list_containers(&self, _all: bool) -> Result<Vec<LocalContainerInfo>> {
        Ok(self.containers.clone())
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_progress_with_all_parts() {
        let line = format_progress_line(Some("abc123"), Some("Pulling fs layer"), Some("[=>  ]"));
        assert_eq!(line, "abc123: Pulling fs layer [=>  ]");
    }

    #[test]
    fn format_progress_without_id() {
        let line = format_progress_line(None, Some("Pulling from library/nginx"), None);
        assert_eq!(line, "Pulling from library/nginx");
    }

    #[test]
    fn format_progress_without_progress() {
        let line = format_progress_line(Some("abc123"), Some("Pull complete"), None);
        assert_eq!(line, "abc123: Pull complete");
    }

    #[test]
    fn format_progress_empty_id_is_omitted() {
        let line = format_progress_line(Some(""), Some("Digest: sha256:abc"), None);
        assert_eq!(line, "Digest: sha256:abc");
    }

    #[test]
    fn format_progress_all_empty_returns_empty() {
        let line = format_progress_line(None, None, None);
        assert!(line.is_empty());
    }

    #[test]
    fn format_progress_only_progress_with_no_status() {
        let line = format_progress_line(Some("abc"), None, Some("[==>]"));
        assert_eq!(line, "abc: [==>]");
    }
}
