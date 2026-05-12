//! Filesystem-watch reload trigger using `notify-debouncer-full`.

use std::path::PathBuf;

use notify_debouncer_full::{DebounceEventResult, new_debouncer, notify::RecursiveMode};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::coordinator::{ReloadRequest, ReloadSource};
use crate::defaults::FILE_WATCH_DEBOUNCE;

/// Spawns a task that watches the directory containing `config_path` and
/// forwards a [`ReloadRequest`] to `tx` whenever `config_path` itself is
/// touched (write, rename-to, or removal).
///
/// Uses a debounce window of [`FILE_WATCH_DEBOUNCE`] to coalesce rapid
/// successive events (e.g. atomic editor saves).
pub fn spawn_file_watch_task(
    config_path: PathBuf,
    tx: mpsc::Sender<ReloadRequest>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Canonicalize so that path comparisons against OS-reported events
        // succeed on platforms where symlinks are involved (e.g. macOS
        // `/var/folders/…` → `/private/var/folders/…`).
        let canonical = match config_path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    error = %e,
                    path = %config_path.display(),
                    "cannot canonicalize config path; falling back to original"
                );
                config_path.clone()
            }
        };

        let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<DebounceEventResult>();

        let mut debouncer = match new_debouncer(FILE_WATCH_DEBOUNCE, None, move |events| {
            // Receiver lives as long as the spawned task; a send failure means
            // the task has already exited, so we silently stop forwarding.
            // Receiver dropped means coordinator is shut down; stop watching.
            drop(notify_tx.send(events));
        }) {
            Ok(d) => d,
            Err(e) => {
                error!(error = ?e, "failed to start file-watch debouncer");
                return;
            }
        };

        let watch_dir = match canonical.parent() {
            Some(p) => p.to_path_buf(),
            None => {
                error!("config path has no parent directory");
                return;
            }
        };

        if let Err(e) = debouncer.watch(&watch_dir, RecursiveMode::NonRecursive) {
            error!(error = ?e, "failed to watch config directory");
            return;
        }

        info!(path = %canonical.display(), "file-watch task started");

        while let Some(batch) = notify_rx.recv().await {
            let events = match batch {
                Ok(e) => e,
                Err(errs) => {
                    for e in errs {
                        error!(error = ?e, "file-watch event error");
                    }
                    continue;
                }
            };

            let touched = events
                .iter()
                .any(|ev| ev.event.paths.iter().any(|p| p == &canonical));

            if !touched {
                continue;
            }

            let req = ReloadRequest {
                source: ReloadSource::FileWatch {
                    path: config_path.clone(),
                },
                timestamp: OffsetDateTime::now_utc(),
            };

            if let Err(e) = tx.send(req).await {
                error!(error = %e, "failed to forward file-watch event to coordinator");
                break;
            }
        }
    })
}
