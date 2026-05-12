//! SIGHUP-based reload trigger.

use time::OffsetDateTime;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::coordinator::{ReloadRequest, ReloadSource};

/// Spawns a task that listens for `SIGHUP` and forwards a [`ReloadRequest`]
/// to `tx` each time the signal is received.
///
/// The task exits when the channel is closed or the SIGHUP stream ends.
pub fn spawn_sighup_task(tx: mpsc::Sender<ReloadRequest>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut sig = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "failed to register SIGHUP handler");
                return;
            }
        };
        while sig.recv().await.is_some() {
            info!("SIGHUP received; enqueueing reload request");
            let req = ReloadRequest {
                source: ReloadSource::Sighup,
                timestamp: OffsetDateTime::now_utc(),
            };
            if let Err(e) = tx.send(req).await {
                error!(error = %e, "failed to forward SIGHUP to coordinator");
                break;
            }
        }
    })
}
