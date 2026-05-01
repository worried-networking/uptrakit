//! Embedded scheduler support for the controller binary.
//!
//! Contains the `CaRotationCheckExecutor` (which needs the in-process CA
//! snapshot watch channel) and the `ControllerSchedulerNotifier` adapter that
//! bridges the engine's `SchedulerNotifier` trait to the controller's
//! `NotificationService`.

use std::sync::Arc;

use tokio::sync::Notify;
use uptrakit_scheduler_engine::{SchedulerNotifier, TaskExecutor};
use uptrakit_shared_db::entity::scheduled_task;
use uptrakit_wire::{ControllerMessage, RequestCaRotationPayload, RequestCrlRenewalPayload};
use uuid::Uuid;

use crate::pki;

#[cfg(feature = "embedded-scheduler")]
pub(crate) struct EmbeddedSchedulerConfig {
    pub db: sea_orm::DatabaseConnection,
    pub notification_service: uptrakit_web_api::notification_service::NotificationService,
    pub controller_id: Uuid,
    pub should_yield: Box<dyn Fn() -> bool + Send + Sync>,
    pub ca_managed: bool,
    pub ca_snapshot: tokio::sync::watch::Receiver<pki::CaSnapshot>,
    pub ca_rotation_trigger: Arc<Notify>,
    pub revocation_notify: Arc<Notify>,
}

#[cfg(feature = "embedded-scheduler")]
pub(crate) async fn run_embedded_scheduler(
    config: EmbeddedSchedulerConfig,
    drain: tokio_util::sync::CancellationToken,
    abort: tokio_util::sync::CancellationToken,
) {
    use uptrakit_scheduler_engine::executors::{
        audit_log_cleanup, crl_renewal, service_cert_check,
    };
    use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;

    let audit_emitter = uptrakit_audit_log::RuntimeAuditEmitter::new();
    let notifier: Arc<dyn SchedulerNotifier> = Arc::new(ControllerSchedulerNotifier::new(
        config.notification_service,
        config.db.clone(),
        Arc::clone(&config.ca_rotation_trigger),
        Arc::clone(&config.revocation_notify),
    ));
    let db = config.db.clone();
    let notifier_for_extras = Arc::clone(&notifier);
    let ca_rotation_trigger = Arc::clone(&config.ca_rotation_trigger);
    let ca_snapshot = config.ca_snapshot;
    let ca_managed = config.ca_managed;

    uptrakit_scheduler_runtime::run_scheduler(
        uptrakit_scheduler_runtime::SchedulerRunConfig::new(
            config.db,
            config.controller_id,
            notifier,
            config.should_yield,
        ),
        drain,
        abort,
        move |scheduler| {
            if ca_managed {
                scheduler.register(
                    ScheduledTaskType::CaRotationCheck,
                    Box::new(CaRotationCheckExecutor::new(
                        ca_snapshot,
                        Arc::clone(&ca_rotation_trigger),
                    )),
                );
            }
            scheduler.register(
                ScheduledTaskType::ServiceCertCheck,
                Box::new(service_cert_check::ServiceCertCheckExecutor::new(
                    db.clone(),
                    Arc::clone(&notifier_for_extras),
                )),
            );
            scheduler.register(
                ScheduledTaskType::CrlRenewal,
                Box::new(crl_renewal::CrlRenewalExecutor::new(Arc::clone(
                    &notifier_for_extras,
                ))),
            );
            scheduler.register(
                ScheduledTaskType::AuditLogCleanup,
                Box::new(audit_log_cleanup::AuditLogCleanupExecutor::new(
                    db.clone(),
                    audit_emitter.clone(),
                )),
            );
            scheduler.register_tick_executor(Box::new(
                uptrakit_scheduler_engine::executors::awaiting_restart::AwaitingRestartExecutor::new(
                    Arc::clone(&notifier_for_extras),
                ),
            ));
        },
    )
    .await;
}

// ---------------------------------------------------------------------------
// CaRotationCheckExecutor
// ---------------------------------------------------------------------------

/// Checks whether the managed CA is within its rotation window and fires the
/// rotation trigger if so.
///
/// This executor stays in the controller binary (not in the engine) because
/// it requires the in-process CA snapshot watch channel and `Notify`.
pub(crate) struct CaRotationCheckExecutor {
    ca_snapshot: tokio::sync::watch::Receiver<pki::CaSnapshot>,
    ca_rotation_trigger: Arc<Notify>,
}

impl CaRotationCheckExecutor {
    pub(crate) fn new(
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
pub(crate) struct ControllerSchedulerNotifier {
    notification_service: uptrakit_web_api::notification_service::NotificationService,
    db: sea_orm::DatabaseConnection,
    ca_rotation_trigger: Arc<Notify>,
    revocation_notify: Arc<Notify>,
}

impl ControllerSchedulerNotifier {
    pub(crate) fn new(
        notification_service: uptrakit_web_api::notification_service::NotificationService,
        db: sea_orm::DatabaseConnection,
        ca_rotation_trigger: Arc<Notify>,
        revocation_notify: Arc<Notify>,
    ) -> Self {
        Self {
            notification_service,
            db,
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

    async fn signal_software_states_changed(&self, tenant_id: uuid::Uuid) {
        self.notification_service
            .push_software_states_for_tenant(&self.db, tenant_id)
            .await;
    }

    async fn signal_crl_renewal(&self) {
        tracing::info!("embedded scheduler triggering CRL rebuild");
        self.revocation_notify.notify_one();
        self.notification_service
            .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                RequestCrlRenewalPayload::default(),
            ))
            .await;
    }

    async fn signal_host_progression(&self, host_id: uuid::Uuid, tenant_id: uuid::Uuid) {
        let dispatch = uptrakit_web_api::queries::update_dispatch::DispatchContext {
            notifier: &self.notification_service,
            protection: None,
        };
        if let Err(e) = uptrakit_web_api::queries::update_batches::dispatch_next_queued_for_host(
            &self.db, dispatch, host_id, tenant_id,
        )
        .await
        {
            tracing::warn!(
                error = %e, %host_id, %tenant_id,
                "signal_host_progression: dispatch_next_queued_for_host failed"
            );
        }
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
            interval_seconds: 86400,
            jitter_seconds: 300,
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
