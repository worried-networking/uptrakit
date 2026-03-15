use std::sync::Arc;

use tokio::sync::mpsc;

use crate::backend::AuditLogBackend;
use crate::entry::AuditEntry;

/// Fire-and-forget audit log dispatcher.
///
/// Audit log entries represent compliance-critical security records and **must never
/// be dropped** due to backpressure. Unlike the notification dispatcher (which uses a
/// bounded channel and drops on overflow), this dispatcher uses an unbounded channel so
/// that every entry is guaranteed to reach the backend as long as the process is running.
///
/// The trade-off is unbounded memory growth if the backend falls severely behind under
/// sustained high load. Operators should monitor backend write latency and scale the DB
/// accordingly. In practice, the background loop drains the channel as fast as the
/// backend can write, so the queue depth should remain near zero under normal conditions.
///
/// Write failures are logged at `warn` level but never surface to event producers.
/// If the channel is closed (dispatcher shut down), the entry is silently dropped.
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
    /// Audit entries are **never** dropped due to backpressure — the channel is unbounded.
    pub fn dispatch(&self, entry: AuditEntry) {
        // UnboundedSender::send only fails when the receiver is dropped (shutdown).
        // Silently discard on shutdown — there is nothing meaningful to do at that point.
        let _ = self.tx.send(entry);
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
