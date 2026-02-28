//! Unix-socket proxy for tunnelling the Docker API over a stdio tunnel.
//!
//! [`DockerSocketProxy`] accepts local Unix-socket connections and bridges
//! each one to `docker system dial-stdio` running on the remote host via
//! [`CommandExecutor::open_stdio_tunnel`]. Bollard then connects to this
//! local socket using its existing `unix://` codepath, avoiding a second SSH
//! connection.
//!
//! This module is only compiled on Unix with the `daemon` feature enabled.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::net::UnixListener;
use uptrakit_plugin_infrastructure_core::command::CommandExecutor;

use crate::error::{DockerError, Result};

/// Monotonic counter for generating unique socket paths per process.
static SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A local Unix socket proxy that bridges each accepted connection to
/// `docker system dial-stdio` on the remote host.
///
/// The proxy is started by [`DockerSocketProxy::start`] and runs until
/// dropped (the listener task is aborted and the socket file removed).
pub(crate) struct DockerSocketProxy {
    socket_path: PathBuf,
    listener_handle: tokio::task::JoinHandle<()>,
}

impl DockerSocketProxy {
    /// Start a new Docker socket proxy backed by the given executor.
    ///
    /// Binds a Unix socket at a unique path under `/tmp/uptrakit/` and spawns
    /// a background task that accepts connections. Each accepted connection
    /// opens a new stdio tunnel to `docker system dial-stdio` and copies
    /// bytes bidirectionally.
    pub(crate) async fn start(executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        let counter = SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
        let socket_dir = std::env::temp_dir().join("uptrakit");

        tokio::fs::create_dir_all(&socket_dir).await.map_err(|e| {
            rootcause::report!(DockerError::DaemonConnection(format!(
                "failed to create proxy socket directory {}: {e}",
                socket_dir.display()
            )))
        })?;

        let socket_path = socket_dir.join(format!(
            "docker-proxy-{}-{counter}.sock",
            std::process::id()
        ));

        // Remove stale socket if it exists from a previous run.
        let _ = tokio::fs::remove_file(&socket_path).await;

        let listener = UnixListener::bind(&socket_path).map_err(|e| {
            rootcause::report!(DockerError::DaemonConnection(format!(
                "failed to bind proxy socket {}: {e}",
                socket_path.display()
            )))
        })?;

        tracing::debug!(
            path = %socket_path.display(),
            "Docker proxy socket listening"
        );

        let path_for_task = socket_path.clone();
        let listener_handle = tokio::spawn(async move {
            Self::accept_loop(listener, executor, &path_for_task).await;
        });

        Ok(Self {
            socket_path,
            listener_handle,
        })
    }

    /// Return the bollard-compatible URI for this proxy socket.
    pub(crate) fn socket_uri(&self) -> String {
        format!("unix://{}", self.socket_path.display())
    }

    /// Accept loop: for each incoming connection, open a stdio tunnel and
    /// bridge the two streams.
    async fn accept_loop(
        listener: UnixListener,
        executor: Arc<dyn CommandExecutor>,
        socket_path: &Path,
    ) {
        loop {
            let (stream, _addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(
                        path = %socket_path.display(),
                        error = %e,
                        "Docker proxy accept failed"
                    );
                    continue;
                }
            };

            let executor = Arc::clone(&executor);
            tokio::spawn(async move {
                match executor.open_stdio_tunnel("docker system dial-stdio").await {
                    Ok(tunnel) => {
                        let (mut stream_read, mut stream_write) =
                            tokio::io::split(stream);
                        let (mut tunnel_read, mut tunnel_write) =
                            tokio::io::split(tunnel);

                        let client_to_docker = tokio::io::copy(&mut stream_read, &mut tunnel_write);
                        let docker_to_client = tokio::io::copy(&mut tunnel_read, &mut stream_write);

                        match tokio::try_join!(client_to_docker, docker_to_client) {
                            Ok((sent, received)) => {
                                tracing::trace!(
                                    sent,
                                    received,
                                    "Docker proxy connection closed"
                                );
                            }
                            Err(e) => {
                                // Connection reset is normal when the client disconnects.
                                if e.kind() != std::io::ErrorKind::ConnectionReset {
                                    tracing::debug!(
                                        error = %e,
                                        "Docker proxy copy error"
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "failed to open stdio tunnel for Docker proxy connection"
                        );
                    }
                }
            });
        }
    }
}

impl Drop for DockerSocketProxy {
    fn drop(&mut self) {
        self.listener_handle.abort();
        // Best-effort cleanup of the socket file.
        let _ = std::fs::remove_file(&self.socket_path);
        tracing::debug!(
            path = %self.socket_path.display(),
            "Docker proxy socket cleaned up"
        );
    }
}
