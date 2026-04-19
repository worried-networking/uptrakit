use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeAuditEvent {
    pub action: String,
    pub level: tracing::Level,
    pub occurred_at: OffsetDateTime,
    pub details: serde_json::Value,
}

pub trait RuntimeAuditForwarder: Send + Sync {
    fn forward(&self, event: &RuntimeAuditEvent);
}

#[derive(Clone, Default)]
pub struct RuntimeAuditEmitter {
    forwarders: Vec<Arc<dyn RuntimeAuditForwarder>>,
}

impl RuntimeAuditEmitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_forwarder(forwarder: Arc<dyn RuntimeAuditForwarder>) -> Self {
        Self {
            forwarders: vec![forwarder],
        }
    }

    pub fn with_additional_forwarder(mut self, forwarder: Arc<dyn RuntimeAuditForwarder>) -> Self {
        self.forwarders.push(forwarder);
        self
    }

    pub fn emit(
        &self,
        action: impl Into<String>,
        level: tracing::Level,
        details: serde_json::Value,
    ) {
        let event = RuntimeAuditEvent {
            action: action.into(),
            level,
            occurred_at: OffsetDateTime::now_utc(),
            details,
        };

        match event.level {
            tracing::Level::ERROR => tracing::error!(
                target: "uptrakit_audit",
                audit_action = %event.action,
                occurred_at = %event.occurred_at,
                details = %event.details,
                "semantic runtime audit event"
            ),
            tracing::Level::WARN => tracing::warn!(
                target: "uptrakit_audit",
                audit_action = %event.action,
                occurred_at = %event.occurred_at,
                details = %event.details,
                "semantic runtime audit event"
            ),
            tracing::Level::INFO => tracing::info!(
                target: "uptrakit_audit",
                audit_action = %event.action,
                occurred_at = %event.occurred_at,
                details = %event.details,
                "semantic runtime audit event"
            ),
            tracing::Level::DEBUG => tracing::debug!(
                target: "uptrakit_audit",
                audit_action = %event.action,
                occurred_at = %event.occurred_at,
                details = %event.details,
                "semantic runtime audit event"
            ),
            tracing::Level::TRACE => tracing::trace!(
                target: "uptrakit_audit",
                audit_action = %event.action,
                occurred_at = %event.occurred_at,
                details = %event.details,
                "semantic runtime audit event"
            ),
        }

        for forwarder in &self.forwarders {
            forwarder.forward(&event);
        }
    }

    pub fn machine_id_validate(
        &self,
        message_name: &str,
        expected: &str,
        received: &str,
        accepted: bool,
    ) {
        self.emit(
            "system.service.machine_id.validate",
            if accepted {
                tracing::Level::INFO
            } else {
                tracing::Level::WARN
            },
            json!({
                "message_name": message_name,
                "expected_machine_id": expected,
                "received_machine_id": received,
                "accepted": accepted,
            }),
        );
    }

    pub fn update_gate(
        &self,
        message_name: &str,
        gate: &str,
        host_machine_id: Option<&str>,
        freeze_file: Option<&Path>,
        cooldown_secs: Option<u64>,
        elapsed_ms: Option<u64>,
    ) {
        self.emit(
            "system.service.update_gate",
            tracing::Level::WARN,
            json!({
                "message_name": message_name,
                "gate": gate,
                "host_machine_id": host_machine_id,
                "freeze_file": freeze_file.map(|path| path.display().to_string()),
                "cooldown_secs": cooldown_secs,
                "elapsed_ms": elapsed_ms,
            }),
        );
    }

    pub fn update_freeze_apply(&self, freeze_file: &Path, enabled: bool, reason: &str) {
        self.emit(
            "system.service.update_freeze.apply",
            tracing::Level::INFO,
            json!({
                "enabled": enabled,
                "freeze_file": freeze_file.display().to_string(),
                "reason": reason,
            }),
        );
    }

    pub fn scheduler_audit_log_cleanup(
        &self,
        tenant_deleted: u64,
        system_deleted: u64,
        retention_days: i64,
    ) {
        self.emit(
            "system.scheduler.audit_log_cleanup",
            tracing::Level::INFO,
            json!({
                "tenant_deleted": tenant_deleted,
                "system_deleted": system_deleted,
                "retention_days": retention_days,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Forwarder {
        events: Mutex<Vec<RuntimeAuditEvent>>,
    }

    impl RuntimeAuditForwarder for Forwarder {
        fn forward(&self, event: &RuntimeAuditEvent) {
            self.events.lock().expect("lock").push(event.clone());
        }
    }

    #[test]
    fn emits_to_forwarder() {
        let forwarder = Arc::new(Forwarder::default());
        let emitter = RuntimeAuditEmitter::with_forwarder(forwarder.clone());

        emitter.update_freeze_apply(Path::new("/tmp/freeze"), true, "test");

        let events = forwarder.events.lock().expect("lock");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "system.service.update_freeze.apply");
        assert_eq!(events[0].level, tracing::Level::INFO);
        assert_eq!(events[0].details["enabled"], true);
    }
}
