//! Docker image pulling abstraction.
//!
//! Defines the [`DockerPuller`] trait for injecting different pull implementations
//! (real bollard client, or mock in tests), and the production [`BollardDockerPuller`]
//! that communicates with the Docker daemon over its Unix socket (or Windows named
//! pipe) via the bollard client library.

use async_trait::async_trait;
use futures_util::StreamExt;
use rootcause::prelude::*;
use uptrakit_provider_core::command::send_output;
use uptrakit_provider_core::mpsc;
use uptrakit_provider_core::{OutputStreamType, UpdateOutputLine};

use crate::config::DockerAuth;
use crate::error::{DockerRegistryError, Result};

// ── Trait ────────────────────────────────────────────────────────────────────

/// Pull a Docker image from a registry.
///
/// The trait exists so that [`crate::provider::DockerRegistryProvider`] can be
/// tested with a mock implementation that does not require a live Docker daemon.
#[async_trait]
pub(crate) trait DockerPuller: Send + Sync {
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
}

// ── Production implementation ────────────────────────────────────────────────

/// Docker image puller backed by the bollard client.
///
/// Communicates with the Docker daemon directly over its Unix socket (or
/// Windows named pipe), with no dependency on the `docker` CLI binary.
pub(crate) struct BollardDockerPuller {
    docker: bollard::Docker,
}

impl BollardDockerPuller {
    /// Create a new puller connected to the local Docker daemon.
    ///
    /// Uses [`bollard::Docker::connect_with_defaults`], which reads
    /// `DOCKER_HOST` and falls back to the platform default socket path.
    /// No actual network connection is opened at construction time.
    pub(crate) fn new() -> Result<Self> {
        let docker = bollard::Docker::connect_with_defaults()
            .context_to::<DockerRegistryError>()?;
        Ok(Self { docker })
    }
}

#[async_trait]
impl DockerPuller for BollardDockerPuller {
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
            let info = item.context_to::<DockerRegistryError>()?;

            // In bollard 0.20, errors are surfaced via `error_detail`.
            if let Some(ref detail) = info.error_detail {
                let msg = detail.message.as_deref().unwrap_or("docker pull error");
                tracing::warn!(error = %msg, "Docker pull stream error");
                bail!(DockerRegistryError::PullFailed(msg.to_string()));
            }

            // `progress` is not a direct String field; format from id + status only.
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
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Map a [`DockerAuth`] config value to bollard credentials.
///
/// For `Basic` auth the server address is inferred from the image reference so
/// the daemon associates the credentials with the correct registry.
fn map_auth_to_credentials(image: &str, auth: &DockerAuth) -> bollard::auth::DockerCredentials {
    match auth {
        DockerAuth::Basic { username, password } => bollard::auth::DockerCredentials {
            username: Some(username.clone()),
            password: Some(password.expose_secret().to_string()),
            serveraddress: Some(infer_serveraddress(image)),
            ..Default::default()
        },
        DockerAuth::Bearer { token } => bollard::auth::DockerCredentials {
            registrytoken: Some(token.expose_secret().to_string()),
            ..Default::default()
        },
    }
}

/// Infer the registry server address from an image reference.
///
/// - `nginx` → `https://index.docker.io/v1/`
/// - `ghcr.io/owner/repo` → `https://ghcr.io/`
/// - `myhost:5000/app` → `https://myhost:5000/`
fn infer_serveraddress(image: &str) -> String {
    if let Some(slash_pos) = image.find('/') {
        let first = &image[..slash_pos];
        if first.contains('.') || first.contains(':') || first == "localhost" {
            return format!("https://{first}/");
        }
    }
    "https://index.docker.io/v1/".to_string()
}

/// Format a single pull-progress event as `"{id}: {status} {progress}"`,
/// omitting any empty components.
fn format_progress_line(
    id: Option<&str>,
    status: Option<&str>,
    progress: Option<&str>,
) -> String {
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
pub(crate) struct MockDockerPuller {
    pub output: String,
    pub should_fail: bool,
}

#[cfg(test)]
#[async_trait]
impl DockerPuller for MockDockerPuller {
    async fn pull_image(
        &self,
        _image: &str,
        _tag: &str,
        _auth: Option<&DockerAuth>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        if self.should_fail {
            bail!(DockerRegistryError::PullFailed("mock pull failure".to_string()));
        }
        send_output(output_tx, &self.output, OutputStreamType::Stdout).await;
        Ok(self.output.clone())
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

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

    #[test]
    fn infer_serveraddress_docker_hub_official() {
        assert_eq!(
            infer_serveraddress("nginx"),
            "https://index.docker.io/v1/"
        );
    }

    #[test]
    fn infer_serveraddress_docker_hub_user() {
        assert_eq!(
            infer_serveraddress("myuser/myrepo"),
            "https://index.docker.io/v1/"
        );
    }

    #[test]
    fn infer_serveraddress_ghcr() {
        assert_eq!(
            infer_serveraddress("ghcr.io/owner/repo"),
            "https://ghcr.io/"
        );
    }

    #[test]
    fn infer_serveraddress_private_with_port() {
        assert_eq!(
            infer_serveraddress("myhost:5000/app"),
            "https://myhost:5000/"
        );
    }

    #[test]
    fn infer_serveraddress_localhost() {
        assert_eq!(
            infer_serveraddress("localhost/app"),
            "https://localhost/"
        );
    }
}
