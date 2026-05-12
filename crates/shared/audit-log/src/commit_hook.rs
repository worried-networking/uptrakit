use std::sync::Arc;

use crate::backend::AuditLogBackend;
use crate::entry::AuditEntryErased;

/// Buffers stateful audit entries that have been written to the DB transaction
/// but not yet mirrored to non-DB backends (e.g. journald). The caller flushes
/// via [`AuditCommitHook::flush_after_commit`] immediately after
/// `tx.commit().await` succeeds.
///
/// Dropping without flushing discards the entries and emits a `tracing::warn!`
/// if any entries were enqueued — turning silent partial delivery into a
/// visible operational signal.
///
/// # Examples
///
/// ```rust
/// # use std::sync::Arc;
/// # use uptrakit_audit_log::backend::NoopBackend;
/// # use uptrakit_audit_log::commit_hook::AuditCommitHook;
/// # async fn example() {
/// let hook = AuditCommitHook::new(Arc::new(NoopBackend));
/// // enqueue entries, then after tx.commit():
/// hook.flush_after_commit().await;
/// # }
/// ```
#[must_use = "call flush_after_commit() after tx.commit() to mirror the audit entry to journald"]
pub struct AuditCommitHook {
    mirror: Arc<dyn AuditLogBackend>,
    pending: parking_lot::Mutex<Vec<AuditEntryErased>>,
    flushed: parking_lot::Mutex<bool>,
}

impl AuditCommitHook {
    /// Creates a new hook that will fan out enqueued entries to `mirror` on flush.
    pub fn new(mirror: Arc<dyn AuditLogBackend>) -> Self {
        Self {
            mirror,
            pending: parking_lot::Mutex::new(Vec::new()),
            flushed: parking_lot::Mutex::new(false),
        }
    }

    /// Adds an entry to the pending queue. Entries are only written to the
    /// mirror backend when [`flush_after_commit`][Self::flush_after_commit] is called.
    pub fn enqueue(&self, entry: AuditEntryErased) {
        self.pending.lock().push(entry);
    }

    /// Consumes the hook and writes all enqueued entries to the mirror backend.
    ///
    /// Call this immediately after `tx.commit().await` succeeds so that
    /// non-DB backends (e.g. journald) receive the entries that were written
    /// inside the now-committed transaction.
    ///
    /// Individual backend failures are logged at `error!` level and do not
    /// propagate — a partial delivery failure must not roll back the
    /// already-committed transaction.
    pub async fn flush_after_commit(self) {
        // Drain inside a braced block so the parking_lot guard is dropped
        // before the loop. Holding a parking_lot::Mutex guard across an
        // `.await` point is forbidden per workspace standards and would make
        // the future !Send.
        let pending = {
            let mut guard = self.pending.lock();
            std::mem::take(&mut *guard)
        };
        *self.flushed.lock() = true;
        for entry in pending {
            if let Err(error) = self.mirror.write(&entry).await {
                tracing::error!(
                    error = %error,
                    action_type = %entry.action_type,
                    "audit commit-hook flush failed"
                );
            }
        }
    }
}

impl Drop for AuditCommitHook {
    fn drop(&mut self) {
        if *self.flushed.lock() {
            return;
        }
        let pending_count = self.pending.lock().len();
        if pending_count > 0 {
            tracing::warn!(
                pending_count,
                "AuditCommitHook dropped with un-flushed entries; \
                 if the surrounding transaction committed, journald missed these rows"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use crate::action_type::{AuditActionKind, AuditActionType};
    use crate::backend::{AuditLogBackend, NoopBackend};
    use crate::entry::{AuditActorType, AuditEntryErased, AuditOutcome};
    use crate::error::AuditLogError;

    use super::AuditCommitHook;

    fn make_stub_erased() -> AuditEntryErased {
        AuditEntryErased {
            id: uuid::Uuid::now_v7(),
            tenant_id: None,
            occurred_at: time::OffsetDateTime::now_utc(),
            actor_type: AuditActorType::System,
            actor_id: None,
            actor_display: None,
            action_type: AuditActionType::AUTH_LOGIN.into(),
            action_kind: AuditActionKind::Event,
            target_type: None,
            target_id: None,
            target_display: None,
            outcome: AuditOutcome::Success,
            details_json: None,
            before_snapshot: None,
            after_snapshot: None,
            correlation_id: None,
            request_id: None,
        }
    }

    struct Counting(Arc<Mutex<usize>>);

    #[async_trait::async_trait]
    impl AuditLogBackend for Counting {
        async fn write(&self, _e: &AuditEntryErased) -> Result<(), AuditLogError> {
            *self.0.lock() += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn commit_hook_flushes_to_backend_only_on_caller_flush() {
        let count = Arc::new(Mutex::new(0_usize));
        let mirror = Arc::new(Counting(Arc::clone(&count)));
        let hook = AuditCommitHook::new(mirror);
        hook.enqueue(make_stub_erased());
        assert_eq!(*count.lock(), 0, "write must not happen before flush");
        hook.flush_after_commit().await;
        assert_eq!(
            *count.lock(),
            1,
            "write must happen exactly once after flush"
        );
    }

    #[tokio::test]
    async fn commit_hook_drops_without_flush_no_panic() {
        let hook = AuditCommitHook::new(Arc::new(NoopBackend));
        // Drop without flush — must not panic even with no pending entries.
        drop(hook);
    }

    #[tokio::test]
    async fn commit_hook_flush_with_empty_queue_marks_flushed() {
        // flush_after_commit with nothing enqueued must still mark as flushed
        // so that Drop does not emit a spurious warning.
        let hook = AuditCommitHook::new(Arc::new(NoopBackend));
        hook.flush_after_commit().await;
        // No panic, no spurious warning path in Drop (flushed = true).
    }

    #[tokio::test]
    async fn commit_hook_multiple_entries_all_written() {
        let count = Arc::new(Mutex::new(0_usize));
        let mirror = Arc::new(Counting(Arc::clone(&count)));
        let hook = AuditCommitHook::new(mirror);
        hook.enqueue(make_stub_erased());
        hook.enqueue(make_stub_erased());
        hook.enqueue(make_stub_erased());
        hook.flush_after_commit().await;
        assert_eq!(*count.lock(), 3, "all three entries must be written");
    }
}
