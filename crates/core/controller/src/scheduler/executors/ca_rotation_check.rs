use std::sync::Arc;

use tokio::sync::Notify;
use uptrakit_shared_db::entity::scheduled_task;

use crate::pki;
use crate::scheduler::executor::TaskExecutor;

/// Checks whether the managed CA is within its rotation window and fires the
/// rotation trigger if so.
///
/// The actual rotation is still handled by the per-controller CA rotation loop
/// (which listens on the `Notify`). This executor only triggers the check on a
/// schedule instead of using a 24h `tokio::time::interval`.
pub struct CaRotationCheckExecutor {
    ca_snapshot: tokio::sync::watch::Receiver<crate::pki::CaSnapshot>,
    ca_rotation_trigger: Arc<Notify>,
}

impl CaRotationCheckExecutor {
    pub fn new(
        ca_snapshot: tokio::sync::watch::Receiver<pki::CaSnapshot>,
        ca_rotation_trigger: Arc<Notify>,
    ) -> Self {
        Self {
            ca_snapshot,
            ca_rotation_trigger,
        }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for CaRotationCheckExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::scheduler::error::Result<()> {
        let snapshot = self.ca_snapshot.borrow().clone();
        if pki::should_rotate_ca(&snapshot.active_cert_pem) {
            tracing::info!("CA certificate is within rotation window, triggering rotation");
            self.ca_rotation_trigger.notify_one();
        } else {
            tracing::debug!("CA certificate does not need rotation");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    /// Build a dummy `scheduled_task::Model` for test use.
    fn dummy_task() -> scheduled_task::Model {
        use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
        scheduled_task::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7(),
            task_type: ScheduledTaskType::CaRotationCheck,
            cron_expression: "0 3 * * *".to_string(),
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: OffsetDateTime::now_utc(),
            locked_by: None,
            locked_at: None,
            last_error: None,
            run_count: 0,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    /// Generate a self-signed CA cert with a specific validity period.
    fn generate_test_ca_cert(days_valid: i64) -> String {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.not_before = OffsetDateTime::now_utc();
        params.not_after = OffsetDateTime::now_utc() + time::Duration::days(days_valid);
        let cert = params.self_signed(&key_pair).expect("self-sign");
        cert.pem()
    }

    fn make_snapshot(cert_pem: String) -> pki::CaSnapshot {
        uptrakit_web_api::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: cert_pem,
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![],
            trusted_ca_cns: vec![],
            bundle_pem: String::new(),
            bundle_hash: String::new(),
            managed: true,
            active_not_after: OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        }
    }

    #[tokio::test]
    async fn does_not_trigger_for_long_lived_ca() {
        // CA valid for 5 years — well outside the 183-day rotation window.
        let cert_pem = generate_test_ca_cert(1825);
        let snapshot = make_snapshot(cert_pem);
        let (tx, rx) = tokio::sync::watch::channel(snapshot);
        let _ = tx; // keep sender alive
        let trigger = Arc::new(Notify::new());

        let executor = CaRotationCheckExecutor::new(rx, trigger.clone());
        executor.execute(&dummy_task()).await.expect("execute");

        // Trigger should not have been notified.
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), trigger.notified()).await;
        assert!(result.is_err(), "should not trigger for long-lived cert");
    }

    #[tokio::test]
    async fn triggers_for_soon_expiring_ca() {
        // CA valid for only 30 days — within the 183-day rotation window.
        let cert_pem = generate_test_ca_cert(30);
        let snapshot = make_snapshot(cert_pem);
        let (tx, rx) = tokio::sync::watch::channel(snapshot);
        let _ = tx;
        let trigger = Arc::new(Notify::new());

        let executor = CaRotationCheckExecutor::new(rx, trigger.clone());
        executor.execute(&dummy_task()).await.expect("execute");

        // Trigger should have been notified.
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), trigger.notified()).await;
        assert!(result.is_ok(), "should trigger for soon-expiring cert");
    }

    #[tokio::test]
    async fn triggers_for_invalid_cert_pem() {
        // Invalid PEM — should_rotate_ca returns true (fail-safe).
        let snapshot = make_snapshot("not a real cert".to_string());
        let (tx, rx) = tokio::sync::watch::channel(snapshot);
        let _ = tx;
        let trigger = Arc::new(Notify::new());

        let executor = CaRotationCheckExecutor::new(rx, trigger.clone());
        executor.execute(&dummy_task()).await.expect("execute");

        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), trigger.notified()).await;
        assert!(result.is_ok(), "should trigger for invalid cert PEM");
    }
}
