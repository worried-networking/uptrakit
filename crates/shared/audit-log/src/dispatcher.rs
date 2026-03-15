use std::sync::Arc;

use tokio::sync::mpsc;

use crate::backend::AuditLogBackend;
use crate::entry::AuditEntry;

/// Channel capacity for the audit log dispatcher.
///
/// Matches the notification dispatcher capacity. When the channel is full
/// (DB backend lagging under high write load), new entries are dropped with
/// a warning rather than blocking producers or growing without bound.
const DISPATCHER_CHANNEL_CAPACITY: usize = 4096;

/// Fire-and-forget audit log dispatcher.
///
/// Follows the same pattern as `NotificationDispatcher`: event producers call
/// `dispatch()` to enqueue entries. The background loop persists entries
/// asynchronously through the configured backend. Write failures are logged
/// but never surface to event producers.
///
/// The internal channel is bounded at [`DISPATCHER_CHANNEL_CAPACITY`] entries.
/// If the backend falls behind and the channel fills, `dispatch()` drops the
/// new entry and logs a warning rather than blocking or panicking.
#[derive(Clone)]
pub struct AuditLogDispatcher {
    tx: mpsc::Sender<AuditEntry>,
}

impl AuditLogDispatcher {
    /// Create a new dispatcher and spawn the background processing loop.
    pub fn new(backend: Arc<dyn AuditLogBackend>) -> Self {
        let (tx, rx) = mpsc::channel(DISPATCHER_CHANNEL_CAPACITY);
        tokio::spawn(dispatch_loop(backend, rx));
        Self { tx }
    }

    /// Enqueue an audit entry for background processing.
    ///
    /// This never blocks and never fails from the caller's perspective.
    /// If the channel is full, the entry is dropped with a warning log.
    /// If the channel is closed (dispatcher shut down), the entry is silently dropped.
    pub fn dispatch(&self, entry: AuditEntry) {
        if let Err(_e) = self.tx.try_send(entry) {
            tracing::warn!("audit log dispatcher channel full, dropping entry");
        }
    }
}

async fn dispatch_loop(backend: Arc<dyn AuditLogBackend>, mut rx: mpsc::Receiver<AuditEntry>) {
    while let Some(entry) = rx.recv().await {
        if let Err(e) = backend.write(&entry).await {
            tracing::warn!(
                audit_id = %entry.id,
                error = %e,
                "failed to write audit log entry"
            );
        }
    }
}
