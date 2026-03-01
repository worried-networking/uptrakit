//! Embedded scheduler support for the controller binary.
//!
//! Contains the `CaRotationCheckExecutor` (which needs the in-process CA
//! snapshot watch channel) and the `ControllerSchedulerNotifier` adapter that
//! bridges the engine's `SchedulerNotifier` trait to the controller's
//! `NotificationService`.

use std::sync::Arc;

use tokio::sync::Notify;
use uptrakit_internal_wire::{
    ControllerMessage, MqttSoftwareStatesPayload, RequestCaRotationPayload,
    RequestCrlRenewalPayload,
};
use uptrakit_scheduler_engine::{SchedulerNotifier, TaskExecutor};
use uptrakit_shared_db::entity::scheduled_task;
use uuid::Uuid;

use crate::pki;

// ---------------------------------------------------------------------------
// CaRotationCheckExecutor
// ---------------------------------------------------------------------------

/// Checks whether the managed CA is within its rotation window and fires the
/// rotation trigger if so.
///
/// This executor stays in the controller binary (not in the engine) because
/// it requires the in-process CA snapshot watch channel and `Notify`.
pub struct CaRotationCheckExecutor {
    ca_snapshot: tokio::sync::watch::Receiver<pki::CaSnapshot>,
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
    async fn execute(
        &self,
        _task: &scheduled_task::Model,
    ) -> uptrakit_scheduler_engine::error::Result<()> {
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

// ---------------------------------------------------------------------------
// ControllerSchedulerNotifier
// ---------------------------------------------------------------------------

/// Bridges the engine's [`SchedulerNotifier`] trait to the controller's
/// [`NotificationService`].
///
/// Used by the embedded scheduler so that engine executors (`VersionCheckExecutor`,
/// `ServiceCertCheckExecutor`) can deliver messages through the same
/// `NotificationService` the controller uses for other WebSocket push messages.
pub struct ControllerSchedulerNotifier {
    notification_service: uptrakit_web_api::notification_service::NotificationService,
    ca_rotation_trigger: Arc<Notify>,
    revocation_notify: Arc<Notify>,
}

impl ControllerSchedulerNotifier {
    pub fn new(
        notification_service: uptrakit_web_api::notification_service::NotificationService,
        ca_rotation_trigger: Arc<Notify>,
        revocation_notify: Arc<Notify>,
    ) -> Self {
        Self {
            notification_service,
            ca_rotation_trigger,
            revocation_notify,
        }
    }
}

#[async_trait::async_trait]
impl SchedulerNotifier for ControllerSchedulerNotifier {
    async fn send_to_service(&self, service_id: &Uuid, msg: ControllerMessage) {
        self.notification_service.send(service_id, msg).await;
    }

    async fn broadcast(&self, msg: ControllerMessage) {
        self.notification_service.broadcast(msg).await;
    }

    async fn send_by_capability(&self, capability: &str, msg: ControllerMessage) {
        self.notification_service
            .send_by_capability(capability, msg)
            .await;
    }

    async fn signal_ca_rotation(&self, reason: &str) {
        tracing::info!(reason, "embedded scheduler requesting CA rotation");
        self.ca_rotation_trigger.notify_one();
        // Also publish to NATS so other controllers can rotate.
        self.notification_service
            .publish_controller_event(ControllerMessage::RequestCaRotation(
                RequestCaRotationPayload {
                    reason: reason.to_string(),
                },
            ))
            .await;
    }

    async fn push_software_states_for_tenant(&self, payload: MqttSoftwareStatesPayload) {
        self.notification_service
            .deliver_software_states(payload)
            .await;
    }

    async fn signal_crl_renewal(&self) {
        tracing::info!("embedded scheduler triggering CRL rebuild");
        self.revocation_notify.notify_one();
        self.notification_service
            .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                RequestCrlRenewalPayload {},
            ))
            .await;
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
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
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
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
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

    #[tokio::test(start_paused = true)]
    async fn does_not_trigger_for_long_lived_ca() {
        let cert_pem = generate_test_ca_cert(1825);
        let snapshot = make_snapshot(cert_pem);
        let (tx, rx) = tokio::sync::watch::channel(snapshot);
        let _ = tx;
        let trigger = Arc::new(Notify::new());

        let executor = CaRotationCheckExecutor::new(rx, trigger.clone());
        executor.execute(&dummy_task()).await.expect("execute");

        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), trigger.notified()).await;
        assert!(result.is_err(), "should not trigger for long-lived cert");
    }

    #[tokio::test(start_paused = true)]
    async fn triggers_for_soon_expiring_ca() {
        let cert_pem = generate_test_ca_cert(30);
        let snapshot = make_snapshot(cert_pem);
        let (tx, rx) = tokio::sync::watch::channel(snapshot);
        let _ = tx;
        let trigger = Arc::new(Notify::new());

        let executor = CaRotationCheckExecutor::new(rx, trigger.clone());
        executor.execute(&dummy_task()).await.expect("execute");

        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), trigger.notified()).await;
        assert!(result.is_ok(), "should trigger for soon-expiring cert");
    }

    #[tokio::test(start_paused = true)]
    async fn triggers_for_invalid_cert_pem() {
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
