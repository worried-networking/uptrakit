//! Docker daemon abstraction.
//!
//! Defines the [`DockerClient`] trait for injecting different implementations
//! (real bollard client in production, mock in tests), and the production
//! [`BollardDockerClient`] that communicates with the Docker daemon.
//!
//! When the `daemon` feature is disabled, only [`NoopDockerClient`] is available.
//! It returns [`DockerError::Configuration`] for every operation, which lets the
//! plugin compile (and serve registry-only capabilities) without pulling in
//! bollard and its TLS stack.

use async_trait::async_trait;
#[cfg(feature = "daemon")]
use futures_util::StreamExt;
use rootcause::prelude::*;
#[cfg(feature = "daemon")]
use std::collections::HashMap;
#[cfg(feature = "daemon")]
use uptrakit_plugin_infrastructure_core::OutputStreamType;
use uptrakit_plugin_infrastructure_core::UpdateOutputLine;
#[cfg(feature = "daemon")]
use uptrakit_plugin_infrastructure_core::command::send_output;
use uptrakit_plugin_infrastructure_core::mpsc;

use crate::config::DockerAuth;
use crate::error::{DockerError, Result};

// ── Public result types ───────────────────────────────────────────────────────

/// Local digest information for an image.
#[derive(Debug, Clone)]
pub struct LocalImageDigest {
    /// The SHA-256 digest string (e.g. `"sha256:abc…"`).
    pub digest: String,
    /// OS field from `ImageInspect.os` (e.g. `"linux"`).
    pub os: Option<String>,
    /// Architecture from `ImageInspect.architecture` (e.g. `"amd64"`, `"arm"`).
    pub architecture: Option<String>,
    /// Architecture variant from `ImageInspect.variant` (e.g. `"v7"` for armv7).
    pub variant: Option<String>,
}

/// Information about a running or stopped container.
#[derive(Debug, Clone)]
pub struct LocalContainerInfo {
    /// The image reference (e.g. `"nginx:latest"` or a bare SHA).
    pub image: String,
    /// Container names (leading `'/'` stripped).
    pub names: Vec<String>,
    /// Container labels (key-value pairs).
    pub labels: std::collections::HashMap<String, String>,
}

/// A container that uses a specific image, with its current run state.
#[derive(Debug, Clone)]
pub struct ContainerForImage {
    /// Container name without the leading `'/'`.
    pub name: String,
    /// `true` if the container is currently running (or paused).
    pub is_running: bool,
    /// Container labels (used to detect compose-managed containers).
    pub labels: std::collections::HashMap<String, String>,
}

// ── Trait ────────────────────────────────────────────────────────────────────

/// Docker daemon operations needed by the Docker plugin.
///
/// The trait exists so that [`crate::plugin::DockerPlugin`] can be
/// tested with a mock implementation that does not require a live Docker daemon.
#[async_trait]
pub(crate) trait DockerClient: Send + Sync {
    /// Ping the Docker daemon to verify it is reachable.
    ///
    /// Returns `Ok(())` when the daemon responds to `GET /_ping`, or an error
    /// if the daemon is unreachable (connection refused, SSH tunnel failure,
    /// etc.). Used by [`crate::plugin::DockerPlugin::detect_host_compatibility`]
    /// to skip discovery on hosts where Docker is not running.
    #[cfg(feature = "daemon")]
    async fn ping(&self) -> Result<()>;

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

    /// List all containers (running and stopped) whose image matches `full_ref`
    /// (e.g. `"nginx:latest"`).
    async fn list_containers_for_image(&self, full_ref: &str) -> Result<Vec<ContainerForImage>>;

    /// Recreate a container in-place, preserving its full configuration.
    ///
    /// Performs: inspect → (stop if running) → remove → create → (start if was_running).
    /// Containers with `AutoRemove = true` are skipped because they manage
    /// their own lifecycle.
    async fn recreate_container(&self, name: &str, was_running: bool) -> Result<()>;
}

// ── Noop implementation (always available) ───────────────────────────────────

/// Stub Docker client that returns [`DockerError::Configuration`] for every
/// operation.
///
/// Used as a default when the `daemon` Cargo feature is disabled so the plugin
/// can compile and serve registry-only capabilities without pulling in bollard
/// and its TLS stack. It is always compiled (unconditionally) and replaced by
/// [`BollardDockerClient`] at runtime when the `daemon` feature is enabled.
pub(crate) struct NoopDockerClient;

#[async_trait]
impl DockerClient for NoopDockerClient {
    #[cfg(feature = "daemon")]
    async fn ping(&self) -> Result<()> {
        bail!(DockerError::Configuration(
            "Docker daemon operations require the 'daemon' Cargo feature".to_string(),
        ))
    }

    async fn pull_image(
        &self,
        _image: &str,
        _tag: &str,
        _auth: Option<&DockerAuth>,
        _output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        bail!(DockerError::Configuration(
            "Docker daemon operations require the 'daemon' Cargo feature".to_string(),
        ))
    }

    async fn inspect_image(&self, _full_ref: &str) -> Result<Option<LocalImageDigest>> {
        bail!(DockerError::Configuration(
            "Docker daemon operations require the 'daemon' Cargo feature".to_string(),
        ))
    }

    async fn list_containers(&self, _all: bool) -> Result<Vec<LocalContainerInfo>> {
        bail!(DockerError::Configuration(
            "Docker daemon operations require the 'daemon' Cargo feature".to_string(),
        ))
    }

    async fn list_containers_for_image(&self, _full_ref: &str) -> Result<Vec<ContainerForImage>> {
        bail!(DockerError::Configuration(
            "Docker daemon operations require the 'daemon' Cargo feature".to_string(),
        ))
    }

    async fn recreate_container(&self, _name: &str, _was_running: bool) -> Result<()> {
        bail!(DockerError::Configuration(
            "Docker daemon operations require the 'daemon' Cargo feature".to_string(),
        ))
    }
}

// ── Production implementation (daemon feature) ──────────────────────────────

/// Probe well-known Docker/Podman Unix socket paths and return the first
/// accessible one, in priority order:
/// 1. `/var/run/docker.sock` (rootful Docker)
/// 2. `/run/user/{euid}/docker.sock` (rootless Docker)
/// 3. `/run/user/{euid}/podman/podman.sock` (rootless Podman)
/// 4. `/run/podman/podman.sock` (rootful Podman)
///
/// Returns `None` when no socket is found (falls back to
/// `bollard::Docker::connect_with_defaults`).
#[cfg(all(unix, feature = "daemon"))]
#[cfg_attr(
    test,
    expect(
        dead_code,
        reason = "used only in non-test daemon connection path; unreachable in test builds"
    )
)]
fn probe_local_socket_path() -> Option<String> {
    use std::os::unix::fs::FileTypeExt;

    // Determine the effective UID for user-scoped socket paths.
    // On Linux this is readable from /proc/self/status; on other Unix
    // systems we skip the user-scoped paths rather than add a libc dep.
    #[cfg(target_os = "linux")]
    let euid: Option<String> = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2)) // effective uid
                .map(|s| s.to_string())
        });
    #[cfg(not(target_os = "linux"))]
    let euid: Option<String> = None;

    let mut candidates = vec!["/var/run/docker.sock".to_string()];
    if let Some(ref uid) = euid {
        candidates.push(format!("/run/user/{uid}/docker.sock"));
        candidates.push(format!("/run/user/{uid}/podman/podman.sock"));
    }
    candidates.push("/run/podman/podman.sock".to_string());

    for path in &candidates {
        if let Ok(meta) = std::fs::metadata(path)
            && meta.file_type().is_socket()
        {
            tracing::debug!(socket = %path, "selected local Docker/Podman socket");
            return Some(path.clone());
        }
    }
    None
}

#[cfg(feature = "daemon")]
/// Docker client backed by the bollard library.
///
/// Communicates with the Docker daemon directly over its Unix socket (or
/// Windows named pipe / TCP / SSH), with no dependency on the `docker` CLI.
pub(crate) struct BollardDockerClient {
    docker: bollard::Docker,
}

#[cfg(feature = "daemon")]
impl BollardDockerClient {
    /// Create a new client connected to the Docker daemon.
    ///
    /// - `None` → use [`bollard::Docker::connect_with_defaults`] (reads
    ///   `DOCKER_HOST`, falls back to the platform default socket).
    /// - `Some("unix:///path")` → connect to that Unix socket.
    /// - `Some("tcp://host:port")` → plain HTTP connection.
    /// - `Some("ssh://user@host[:port]")` → SSH tunnel (**requires the `ssh`
    ///   Cargo feature**).
    pub(crate) fn new(
        docker_host: Option<&str>,
        ssh_key_path: Option<&str>,
        tls: Option<&crate::config::DockerTlsConfig>,
    ) -> Result<Self> {
        let docker = Self::connect(docker_host, ssh_key_path, tls)?;
        Ok(Self { docker })
    }

    #[expect(
        clippy::string_slice,
        reason = "slice uses fixed-length ASCII prefix len; 'unix://' is 7 ASCII bytes so the offset is always a valid char boundary"
    )]
    fn connect(
        docker_host: Option<&str>,
        ssh_key_path: Option<&str>,
        tls: Option<&crate::config::DockerTlsConfig>,
    ) -> Result<bollard::Docker> {
        use bollard::API_DEFAULT_VERSION;
        // Use a shorter timeout in test builds so that tests that probe the
        // local Docker daemon do not block for 2 minutes when the daemon is
        // not reachable (e.g. different socket path, CI without Docker, etc.).
        #[cfg(not(test))]
        const TIMEOUT: u64 = 120;
        #[cfg(test)]
        const TIMEOUT: u64 = 5;

        match docker_host {
            None => {
                #[cfg(all(unix, not(test)))]
                if let Some(socket_path) = probe_local_socket_path() {
                    return bollard::Docker::connect_with_socket(
                        &socket_path,
                        TIMEOUT,
                        API_DEFAULT_VERSION,
                    )
                    .context_to::<DockerError>();
                }
                bollard::Docker::connect_with_defaults().context_to::<DockerError>()
            }

            Some(h) if h.starts_with("unix://") => {
                let path = &h["unix://".len()..];
                bollard::Docker::connect_with_socket(path, TIMEOUT, API_DEFAULT_VERSION)
                    .context_to::<DockerError>()
            }

            // SSH connector — only compiled when the `ssh` Cargo feature is
            // enabled (openssh crate, system ssh binary with docker
            // system-dial-stdio).  Pass the key path directly so the stored
            // per-host key is used rather than falling back to ~/.ssh/.
            #[cfg(feature = "ssh")]
            Some(h) if h.starts_with("ssh://") => bollard::Docker::connect_with_ssh(
                h,
                TIMEOUT,
                API_DEFAULT_VERSION,
                ssh_key_path.map(str::to_string),
            )
            .context_to::<DockerError>(),

            Some(h) => {
                // Give a clear error for SSH URLs when the `ssh` feature is
                // disabled, rather than silently falling through to an HTTP
                // connection attempt that would produce a confusing error.
                if h.starts_with("ssh://") {
                    let _ = ssh_key_path;
                    bail!(DockerError::Configuration(
                        "SSH Docker connections require the 'ssh' Cargo feature to be enabled"
                            .to_string()
                    ));
                }
                // Use TLS when configured for TCP connections.
                if (h.starts_with("tcp://") || h.starts_with("http://"))
                    && let Some(tls_cfg) = tls
                {
                    use std::path::Path;
                    return bollard::Docker::connect_with_ssl(
                        h,
                        Path::new(tls_cfg.client_key_path.as_deref().unwrap_or("")),
                        Path::new(tls_cfg.client_cert_path.as_deref().unwrap_or("")),
                        Path::new(tls_cfg.ca_cert_path.as_deref().unwrap_or("")),
                        TIMEOUT,
                        API_DEFAULT_VERSION,
                    )
                    .context_to::<DockerError>();
                }
                bollard::Docker::connect_with_http(h, TIMEOUT, API_DEFAULT_VERSION)
                    .context_to::<DockerError>()
            }
        }
    }
}

#[cfg(feature = "daemon")]
#[async_trait]
impl DockerClient for BollardDockerClient {
    async fn ping(&self) -> Result<()> {
        self.docker.ping().await.context_to::<DockerError>()?;
        Ok(())
    }

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

        let mut tracker = PullProgressTracker::new();

        while let Some(item) = stream.next().await {
            let info = item.context_to::<DockerError>()?;

            if let Some(ref detail) = info.error_detail {
                let msg = detail.message.as_deref().unwrap_or("docker pull error");
                tracing::warn!(error = %msg, "Docker pull stream error");
                bail!(DockerError::PullFailed(msg.to_string()));
            }

            if let Some(frame) = tracker.handle_event(&info) {
                send_output(output_tx, &frame, OutputStreamType::Stdout).await;
            }
        }

        tracing::debug!("Docker image pull stream completed");
        Ok(tracker.into_clean_output())
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

                Ok(digest.map(|d| LocalImageDigest {
                    digest: d,
                    os: info.os.clone(),
                    architecture: info.architecture.clone(),
                    variant: info.variant.clone(),
                }))
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
                labels: c.labels.unwrap_or_default(),
            })
            .collect();

        Ok(infos)
    }

    async fn list_containers_for_image(&self, full_ref: &str) -> Result<Vec<ContainerForImage>> {
        use bollard::models::ContainerSummaryStateEnum;
        use bollard::query_parameters::ListContainersOptions;
        use std::collections::HashMap;

        let filters = Some(HashMap::from([(
            "ancestor".to_string(),
            vec![full_ref.to_string()],
        )]));

        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            }))
            .await
            .context_to::<DockerError>()?;

        let result = containers
            .into_iter()
            .filter_map(|c| {
                let name = c
                    .names
                    .as_deref()
                    .and_then(|ns| ns.first())
                    .map(|n| n.trim_start_matches('/').to_string())?;

                let is_running = matches!(
                    c.state,
                    Some(ContainerSummaryStateEnum::RUNNING)
                        | Some(ContainerSummaryStateEnum::PAUSED)
                );

                let labels = c.labels.unwrap_or_default();

                Some(ContainerForImage {
                    name,
                    is_running,
                    labels,
                })
            })
            .collect();

        Ok(result)
    }

    async fn recreate_container(&self, name: &str, was_running: bool) -> Result<()> {
        use bollard::models::NetworkingConfig;
        use bollard::query_parameters::{
            CreateContainerOptions, RemoveContainerOptions, StopContainerOptions,
        };

        let inspect = self
            .docker
            .inspect_container(name, None)
            .await
            .context_to::<DockerError>()?;

        // Skip auto-remove containers — they manage their own lifecycle.
        if inspect
            .host_config
            .as_ref()
            .and_then(|hc| hc.auto_remove)
            .unwrap_or(false)
        {
            tracing::debug!(
                container = %name,
                "skipping auto-remove container recreation"
            );
            return Ok(());
        }

        if was_running {
            tracing::debug!(container = %name, "stopping container before recreation");
            self.docker
                .stop_container(name, None::<StopContainerOptions>)
                .await
                .context_to::<DockerError>()?;
        }

        tracing::debug!(container = %name, "removing container for recreation");
        self.docker
            .remove_container(
                name,
                Some(RemoveContainerOptions {
                    force: false,
                    ..Default::default()
                }),
            )
            .await
            .context_to::<DockerError>()?;

        // Build the create body from the inspected configuration using a JSON
        // round-trip. `ContainerConfig` and `ContainerCreateBody` share identical
        // JSON field names, so deserialising one from the other's serialised
        // form preserves every shared field automatically — including any new
        // fields added in future bollard versions — without any manual mapping.
        let config = inspect.config.unwrap_or_default();
        let config_json = serde_json::to_value(&config).map_err(|e| {
            report!(DockerError::Configuration(format!(
                "failed to serialize container config: {e}"
            )))
        })?;
        let mut body: bollard::models::ContainerCreateBody = serde_json::from_value(config_json)
            .map_err(|e| {
                report!(DockerError::Configuration(format!(
                    "failed to deserialize container config: {e}"
                )))
            })?;

        body.host_config = inspect.host_config;
        body.networking_config =
            inspect
                .network_settings
                .and_then(|ns| ns.networks)
                .map(|networks| NetworkingConfig {
                    endpoints_config: Some(networks),
                });

        tracing::debug!(container = %name, "creating container from saved config");
        self.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(name.to_string()),
                    ..Default::default()
                }),
                body,
            )
            .await
            .context_to::<DockerError>()?;

        if was_running {
            tracing::debug!(container = %name, "starting recreated container");
            self.docker
                .start_container(
                    name,
                    None::<bollard::query_parameters::StartContainerOptions>,
                )
                .await
                .context_to::<DockerError>()?;
        }

        tracing::info!(
            container = %name,
            was_running,
            "container recreated successfully"
        );
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Map a [`DockerAuth`] config value to bollard credentials.
#[cfg(feature = "daemon")]
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
#[cfg(all(test, feature = "daemon"))]
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

/// Format a byte count as a human-readable string (B, KB, MB, GB).
#[cfg(feature = "daemon")]
fn format_bytes(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2}GB", b / GB)
    } else if b >= MB {
        format!("{:.2}MB", b / MB)
    } else if b >= KB {
        format!("{:.2}KB", b / KB)
    } else {
        format!("{bytes}B")
    }
}

/// Build a progress bar string like `[======>                       ]`.
///
/// `width` is the number of characters inside the brackets.
#[cfg(feature = "daemon")]
fn format_progress_bar(current: i64, total: i64, width: usize) -> String {
    if total <= 0 {
        return format!("[{}]", " ".repeat(width));
    }
    let ratio = (current as f64 / total as f64).clamp(0.0, 1.0);
    let filled = (ratio * width as f64).round() as usize;
    let filled = filled.min(width);

    let mut bar = String::with_capacity(width + 2);
    bar.push('[');
    if filled > 0 {
        // filled - 1 chars of '=' then '>'
        for _ in 0..filled.saturating_sub(1) {
            bar.push('=');
        }
        bar.push('>');
    }
    for _ in filled..width {
        bar.push(' ');
    }
    bar.push(']');
    bar
}

// ── Pull progress tracker ────────────────────────────────────────────────────

/// Minimum interval between ANSI frame emissions for progress-only updates.
#[cfg(feature = "daemon")]
const MIN_FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);

/// Width of the progress bar (characters inside brackets).
#[cfg(feature = "daemon")]
const BAR_WIDTH: usize = 30;

/// Terminal statuses that are always emitted immediately (no throttle).
#[cfg(feature = "daemon")]
const TERMINAL_STATUSES: &[&str] = &[
    "Pull complete",
    "Already exists",
    "Download complete",
    "Verifying Checksum",
    "Extracting",
];

/// Per-layer state tracked during a Docker pull operation.
#[cfg(feature = "daemon")]
struct LayerState {
    status: String,
    current: i64,
    total: i64,
}

/// Tracks Docker pull progress events and produces ANSI escape sequences for
/// in-place terminal redrawing, mimicking `docker pull` output.
///
/// One-off messages (no layer ID) are emitted as plain lines.
/// Layer events are collected and rendered as a block of lines that is
/// redrawn in-place using cursor-up + line-clear sequences.
///
/// The output travels through MPSC → WebSocket → DB → SSE → xterm.js,
/// which natively handles the ANSI sequences.
#[cfg(feature = "daemon")]
struct PullProgressTracker {
    /// Layer IDs in the order they first appeared.
    layer_order: Vec<String>,
    /// Current state per layer.
    layer_state: HashMap<String, LayerState>,
    /// Number of terminal lines currently occupied by the layer block.
    displayed_lines: usize,
    /// Timestamp of the last emitted ANSI frame (for throttling).
    last_frame_at: Option<tokio::time::Instant>,
    /// One-off messages accumulated for clean output.
    messages: Vec<String>,
}

#[cfg(feature = "daemon")]
impl PullProgressTracker {
    fn new() -> Self {
        Self {
            layer_order: Vec::new(),
            layer_state: HashMap::new(),
            displayed_lines: 0,
            last_frame_at: None,
            messages: Vec::new(),
        }
    }

    /// Process a single Docker pull event and return an ANSI frame to emit,
    /// or `None` if the event was throttled.
    fn handle_event(&mut self, info: &bollard::models::CreateImageInfo) -> Option<String> {
        let status = info.status.as_deref().unwrap_or("").trim();
        let id = info.id.as_deref().map(str::trim);

        match id {
            None | Some("") => self.handle_message(status),
            Some(layer_id) => self.handle_layer_event(layer_id, status, &info.progress_detail),
        }
    }

    /// Handle a one-off message (no layer ID).
    fn handle_message(&mut self, status: &str) -> Option<String> {
        if status.is_empty() {
            return None;
        }

        let mut out = String::new();

        // If we have displayed layer lines, move cursor below the block first.
        if self.displayed_lines > 0 {
            out.push('\n');
            self.displayed_lines = 0;
        }

        out.push_str(status);
        out.push('\n');

        self.messages.push(status.to_string());
        Some(out)
    }

    /// Handle a per-layer event and optionally build an ANSI frame.
    fn handle_layer_event(
        &mut self,
        layer_id: &str,
        status: &str,
        progress_detail: &Option<bollard::models::ProgressDetail>,
    ) -> Option<String> {
        if status.is_empty() {
            return None;
        }

        // Track this layer if new.
        if !self.layer_state.contains_key(layer_id) {
            self.layer_order.push(layer_id.to_string());
        }

        let (current, total) = progress_detail
            .as_ref()
            .map(|d| (d.current.unwrap_or(0), d.total.unwrap_or(0)))
            .unwrap_or((0, 0));

        let previous_status = self
            .layer_state
            .get(layer_id)
            .map(|s| s.status.clone())
            .unwrap_or_default();

        self.layer_state.insert(
            layer_id.to_string(),
            LayerState {
                status: status.to_string(),
                current,
                total,
            },
        );

        let status_changed = previous_status != status;
        let is_terminal = TERMINAL_STATUSES
            .iter()
            .any(|s| status.eq_ignore_ascii_case(s));

        // Throttle: always emit on status change or terminal status;
        // otherwise rate-limit progress-only updates.
        if !status_changed
            && !is_terminal
            && let Some(last) = self.last_frame_at
            && last.elapsed() < MIN_FRAME_INTERVAL
        {
            return None;
        }

        let frame = self.build_frame();
        self.last_frame_at = Some(tokio::time::Instant::now());
        Some(frame)
    }

    /// Build an ANSI frame that redraws all layer lines in-place.
    #[expect(
        clippy::unused_result_ok,
        reason = "write! to String always succeeds; .ok() is used to discard the infallible fmt::Error"
    )]
    fn build_frame(&mut self) -> String {
        let layer_count = self.layer_order.len();
        let mut out = String::with_capacity(layer_count * 80);

        // Move cursor up to the first layer line (overwrite previous frame).
        if self.displayed_lines > 0 {
            write!(out, "\x1b[{}A", self.displayed_lines).ok();
        }

        for layer_id in &self.layer_order {
            if let Some(state) = self.layer_state.get(layer_id) {
                // \r = carriage return, \x1b[2K = clear entire line
                out.push_str("\r\x1b[2K");
                out.push_str(layer_id);
                out.push_str(": ");
                out.push_str(&state.status);

                // Show progress bar for statuses with meaningful byte counts.
                if state.total > 0 {
                    out.push_str("  ");
                    out.push_str(&format_progress_bar(state.current, state.total, BAR_WIDTH));
                    out.push_str("  ");
                    out.push_str(&format_bytes(state.current));
                    out.push('/');
                    out.push_str(&format_bytes(state.total));
                }

                out.push('\n');
            }
        }

        self.displayed_lines = layer_count;
        out
    }

    /// Consume the tracker and return clean (ANSI-free) output for storage
    /// in `update_history.output`.
    fn into_clean_output(self) -> String {
        let mut out = String::new();

        for msg in &self.messages {
            out.push_str(msg);
            out.push('\n');
        }

        for layer_id in &self.layer_order {
            if let Some(state) = self.layer_state.get(layer_id) {
                out.push_str(layer_id);
                out.push_str(": ");
                out.push_str(&state.status);
                out.push('\n');
            }
        }

        out
    }
}

/// `write!` macro support for building ANSI frames.
#[cfg(feature = "daemon")]
use std::fmt::Write;

// ── Test mock ────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "daemon"))]
#[derive(Default)]
pub(crate) struct MockDockerClient {
    pub pull_output: String,
    pub pull_should_fail: bool,
    pub ping_should_fail: bool,
    /// When `true`, `ping()` sleeps indefinitely so that timeout tests can
    /// advance virtual time and verify that the caller's timeout fires.
    pub ping_should_hang: bool,
    pub inspect_result: Option<String>, // Some(digest) or None
    /// Optional OS string returned in `LocalImageDigest.os` (e.g. `"linux"`).
    /// Used by tests that need platform metadata without making real registry calls.
    pub inspect_os: Option<String>,
    /// Optional architecture string returned in `LocalImageDigest.architecture`
    /// (e.g. `"amd64"`, `"arm"`).
    pub inspect_architecture: Option<String>,
    /// Optional variant string returned in `LocalImageDigest.variant` (e.g. `"v7"`).
    pub inspect_variant: Option<String>,
    pub containers: Vec<LocalContainerInfo>,
    pub containers_for_image: Vec<ContainerForImage>,
    pub recreate_should_fail: bool,
}

#[cfg(all(test, feature = "daemon"))]
#[async_trait]
impl DockerClient for MockDockerClient {
    async fn ping(&self) -> Result<()> {
        if self.ping_should_hang {
            // Sleep "forever" under virtual time; the caller's timeout cancels
            // this future before Duration::MAX is ever reached.
            tokio::time::sleep(std::time::Duration::MAX).await;
        }
        if self.ping_should_fail {
            bail!(DockerError::DaemonConnection(
                "mock ping failure".to_string()
            ));
        }
        Ok(())
    }

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
        Ok(self.inspect_result.clone().map(|d| LocalImageDigest {
            digest: d,
            os: self.inspect_os.clone(),
            architecture: self.inspect_architecture.clone(),
            variant: self.inspect_variant.clone(),
        }))
    }

    async fn list_containers(&self, _all: bool) -> Result<Vec<LocalContainerInfo>> {
        Ok(self.containers.clone())
    }

    async fn list_containers_for_image(&self, _full_ref: &str) -> Result<Vec<ContainerForImage>> {
        Ok(self.containers_for_image.clone())
    }

    async fn recreate_container(&self, _name: &str, _was_running: bool) -> Result<()> {
        if self.recreate_should_fail {
            bail!(DockerError::DaemonConnection(
                "mock recreate failure".to_string()
            ));
        }
        Ok(())
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "daemon"))]
mod tests {
    use super::*;

    // ── format_progress_line tests ───────────────────────────────────────

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

    // ── format_bytes tests ──────────────────────────────────────────────

    #[test]
    fn format_bytes_values() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(512), "512B");
        assert_eq!(format_bytes(1024), "1.00KB");
        assert_eq!(format_bytes(1536), "1.50KB");
        assert_eq!(format_bytes(1_048_576), "1.00MB");
        assert_eq!(format_bytes(10_485_760), "10.00MB");
        assert_eq!(format_bytes(1_073_741_824), "1.00GB");
    }

    // ── format_progress_bar tests ───────────────────────────────────────

    #[test]
    fn progress_bar_zero_total_shows_empty() {
        let bar = format_progress_bar(100, 0, 10);
        assert_eq!(bar, "[          ]");
    }

    #[test]
    fn progress_bar_half_filled() {
        let bar = format_progress_bar(500, 1000, 10);
        assert_eq!(bar, "[====>     ]");
    }

    #[test]
    fn progress_bar_complete() {
        let bar = format_progress_bar(1000, 1000, 10);
        assert_eq!(bar, "[=========>]");
    }

    #[test]
    fn progress_bar_start() {
        let bar = format_progress_bar(0, 1000, 10);
        assert_eq!(bar, "[          ]");
    }

    // ── PullProgressTracker tests ───────────────────────────────────────

    fn make_message_event(status: &str) -> bollard::models::CreateImageInfo {
        bollard::models::CreateImageInfo {
            id: None,
            status: Some(status.to_string()),
            progress_detail: None,
            error_detail: None,
        }
    }

    fn make_layer_event(
        id: &str,
        status: &str,
        current: i64,
        total: i64,
    ) -> bollard::models::CreateImageInfo {
        bollard::models::CreateImageInfo {
            id: Some(id.to_string()),
            status: Some(status.to_string()),
            progress_detail: Some(bollard::models::ProgressDetail {
                current: Some(current),
                total: Some(total),
            }),
            error_detail: None,
        }
    }

    fn make_layer_event_no_progress(id: &str, status: &str) -> bollard::models::CreateImageInfo {
        bollard::models::CreateImageInfo {
            id: Some(id.to_string()),
            status: Some(status.to_string()),
            progress_detail: None,
            error_detail: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tracker_emits_one_off_messages() {
        let mut tracker = PullProgressTracker::new();
        let event = make_message_event("Pulling from library/nginx");
        let output = tracker.handle_event(&event);
        assert!(output.is_some());
        let text = output.unwrap();
        assert!(text.contains("Pulling from library/nginx"));
        assert!(text.ends_with('\n'));
    }

    #[tokio::test(start_paused = true)]
    async fn tracker_emits_layer_events_with_ansi() {
        let mut tracker = PullProgressTracker::new();
        let event = make_layer_event("abc123", "Downloading", 500, 1000);
        let output = tracker.handle_event(&event);
        assert!(output.is_some());
        let text = output.unwrap();
        // Should contain the layer ID, status, progress bar, and byte counts.
        assert!(text.contains("abc123"));
        assert!(text.contains("Downloading"));
        assert!(text.contains('['));
        assert!(text.contains("500B/1000B"));
    }

    #[tokio::test(start_paused = true)]
    async fn tracker_redraws_with_cursor_up() {
        let mut tracker = PullProgressTracker::new();

        // First event — no cursor-up needed.
        let event1 = make_layer_event("abc", "Downloading", 100, 1000);
        let out1 = tracker.handle_event(&event1).unwrap();
        // First frame has no cursor-up escape (no \x1b[<N>A pattern).
        assert!(
            !out1.contains("\x1b[1A"),
            "first frame should not move cursor up"
        );

        // Status change on same layer — cursor-up by 1.
        let event2 = make_layer_event_no_progress("abc", "Pull complete");
        let out2 = tracker.handle_event(&event2).unwrap();
        assert!(
            out2.contains("\x1b[1A"),
            "second frame should move cursor up by 1"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tracker_multiple_layers_cursor_up() {
        let mut tracker = PullProgressTracker::new();

        let e1 = make_layer_event("aaa", "Downloading", 100, 1000);
        let e2 = make_layer_event("bbb", "Downloading", 200, 2000);
        tracker.handle_event(&e1);
        // Second layer appears — status change so always emitted.
        let out = tracker.handle_event(&e2).unwrap();
        // displayed_lines was 1 after first event, so cursor-up by 1.
        assert!(out.contains("\x1b[1A"));
        assert!(out.contains("aaa"));
        assert!(out.contains("bbb"));

        // Now status change on first layer — cursor-up by 2.
        let e3 = make_layer_event_no_progress("aaa", "Pull complete");
        let out3 = tracker.handle_event(&e3).unwrap();
        assert!(out3.contains("\x1b[2A"));
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_suppresses_rapid_progress_updates() {
        let mut tracker = PullProgressTracker::new();
        let event1 = make_layer_event("abc", "Downloading", 100, 1000);
        let event2 = make_layer_event("abc", "Downloading", 200, 1000);
        let event3 = make_layer_event("abc", "Downloading", 300, 1000);

        assert!(
            tracker.handle_event(&event1).is_some(),
            "first event always emitted"
        );
        assert!(
            tracker.handle_event(&event2).is_none(),
            "rapid update throttled"
        );

        tokio::time::advance(std::time::Duration::from_millis(200)).await;

        assert!(
            tracker.handle_event(&event3).is_some(),
            "emitted after interval elapsed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn status_change_bypasses_throttle() {
        let mut tracker = PullProgressTracker::new();
        let event1 = make_layer_event("abc", "Downloading", 100, 1000);
        let event2 = make_layer_event_no_progress("abc", "Pull complete");

        assert!(tracker.handle_event(&event1).is_some());
        // Immediately after — status changed, so must emit.
        assert!(
            tracker.handle_event(&event2).is_some(),
            "status change always emits"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn terminal_status_bypasses_throttle() {
        let mut tracker = PullProgressTracker::new();
        let e1 = make_layer_event("abc", "Already exists", 0, 0);
        assert!(
            tracker.handle_event(&e1).is_some(),
            "terminal status emits immediately"
        );

        // Same terminal status again — still a terminal status, bypasses throttle.
        let e2 = make_layer_event("abc", "Already exists", 0, 0);
        // Status didn't change but "Already exists" is terminal.
        assert!(
            tracker.handle_event(&e2).is_some(),
            "terminal status always emits"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn message_between_layers_seals_block() {
        let mut tracker = PullProgressTracker::new();
        let layer = make_layer_event("abc", "Downloading", 100, 1000);
        tracker.handle_event(&layer);
        assert_eq!(tracker.displayed_lines, 1);

        let msg = make_message_event("Digest: sha256:abc123");
        let out = tracker.handle_event(&msg).unwrap();
        assert!(
            out.starts_with('\n'),
            "message after layers should start with newline to seal block"
        );
        assert_eq!(tracker.displayed_lines, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn clean_output_contains_final_states() {
        let mut tracker = PullProgressTracker::new();

        let msg1 = make_message_event("Pulling from library/nginx");
        tracker.handle_event(&msg1);

        let e1 = make_layer_event("abc", "Downloading", 500, 1000);
        tracker.handle_event(&e1);
        let e2 = make_layer_event_no_progress("abc", "Pull complete");
        tracker.handle_event(&e2);

        let msg2 = make_message_event("Digest: sha256:deadbeef");
        tracker.handle_event(&msg2);

        let clean = tracker.into_clean_output();
        assert!(clean.contains("Pulling from library/nginx"));
        assert!(clean.contains("abc: Pull complete"));
        assert!(clean.contains("Digest: sha256:deadbeef"));
        // No ANSI escape sequences in clean output.
        assert!(!clean.contains("\x1b["));
        assert!(!clean.contains("\x1b[2K"));
    }

    #[tokio::test(start_paused = true)]
    async fn empty_status_events_are_skipped() {
        let mut tracker = PullProgressTracker::new();
        let event = make_message_event("");
        assert!(tracker.handle_event(&event).is_none());

        let layer_event = make_layer_event_no_progress("abc", "");
        assert!(tracker.handle_event(&layer_event).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn layer_without_progress_detail_shows_no_bar() {
        let mut tracker = PullProgressTracker::new();
        let event = make_layer_event_no_progress("abc", "Waiting");
        let out = tracker.handle_event(&event).unwrap();
        assert!(out.contains("abc: Waiting"));
        // No progress bar when there's no byte total (the `[===>` pattern).
        assert!(
            !out.contains("[=") && !out.contains("[>") && !out.contains("[ "),
            "should not contain a progress bar: {out}"
        );
    }
}
