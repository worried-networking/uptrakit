//! Boundary trait for writing operational alerts from the reload coordinator.
//!
//! `config-reload` is ignorant of `audit-log`; the adapter lives in
//! `controller-runtime` which depends on both crates.

/// Severity of an operational system alert.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertSeverity {
    /// Degraded but operational — operator should investigate soon.
    Warning,
    /// Operation failed; config was rolled back.
    Error,
    /// Coordinator entered Degraded state; manual recovery required.
    Critical,
}

impl AlertSeverity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }
}

/// Sink for operational alerts emitted by the reload coordinator.
///
/// Implemented by the controller-runtime adapter that delegates to
/// [`uptrakit_audit_log::AuditEmitter`].  Tests may use [`NoopAlertWriter`].
#[async_trait::async_trait]
pub trait SystemAlertWriter: Send + Sync {
    async fn write(&self, severity: AlertSeverity, message: String);
}

/// No-op implementation — discards all alerts silently.
pub struct NoopAlertWriter;

#[async_trait::async_trait]
impl SystemAlertWriter for NoopAlertWriter {
    async fn write(&self, _severity: AlertSeverity, _message: String) {}
}
