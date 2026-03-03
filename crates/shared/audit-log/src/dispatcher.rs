use std::sync::Arc;

use tokio::sync::mpsc;

use crate::backend::AuditLogBackend;
use crate::entry::AuditEntry;

/// Fire-and-forget audit log dispatcher.
///
/// Follows the same pattern as `NotificationDispatcher`: event producers call
/// `dispatch()` to enqueue entries. The background loop persists entries
/// asynchronously through the configured backend. Write failures are logged
/// but never surface to event producers.
#[derive(Clone)]
pub struct AuditLogDispatcher {
    tx: mpsc::UnboundedSender<AuditEntry>,
}

impl AuditLogDispatcher {
    /// Create a new dispatcher and spawn the background processing loop.
    pub fn new(backend: Arc<dyn AuditLogBackend>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(dispatch_loop(backend, rx));
        Self { tx }
    }

    /// Enqueue an audit entry for background processing.
    ///
    /// This never blocks and never fails from the caller's perspective.
    /// If the channel is closed (dispatcher shut down), the entry is silently dropped.
    pub fn dispatch(&self, entry: AuditEntry) {
        if let Err(e) = self.tx.send(entry) {
            tracing::warn!(
                "audit log dispatcher channel closed, dropping entry: {}",
                e
            );
        }
    }
}

async fn dispatch_loop(
    backend: Arc<dyn AuditLogBackend>,
    mut rx: mpsc::UnboundedReceiver<AuditEntry>,
) {
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
