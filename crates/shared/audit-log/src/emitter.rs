use crate::dispatcher::AuditLogDispatcher;
use crate::entry::AuditEntry;

#[derive(Clone)]
pub struct AuditEmitter {
    dispatcher: AuditLogDispatcher,
}

impl AuditEmitter {
    pub fn new(dispatcher: AuditLogDispatcher) -> Self {
        Self { dispatcher }
    }

    pub fn emit_best_effort(&self, entry: AuditEntry) {
        if let Err(err) = entry.validate() {
            tracing::warn!(error = %err, "dropping invalid audit entry");
            return;
        }
        self.dispatcher.dispatch(entry);
    }
}
