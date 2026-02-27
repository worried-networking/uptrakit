//! External CA rotation check executor.
//!
//! Reads the active CA certificate from the database and checks whether it is
//! within the rotation window. If so, signals the controller(s) via NATS.
//!
//! This lives in the external scheduler binary (not the engine) because the
//! embedded scheduler uses the in-process CA watch channel instead.

use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter, QueryOrder};
use uptrakit_scheduler_engine::ca_utils;
use uptrakit_scheduler_engine::error::SchedulerError;
use uptrakit_scheduler_engine::{SchedulerNotifier, TaskExecutor};
use uptrakit_shared_db::entity::{ca_certificate, scheduled_task};

/// Checks the active CA certificate from the database and signals rotation
/// to controllers via NATS when the certificate is within the rotation window.
pub struct ExternalCaRotationCheckExecutor {
    db: DatabaseConnection,
    notifier: Arc<dyn SchedulerNotifier>,
}

impl ExternalCaRotationCheckExecutor {
    pub fn new(db: DatabaseConnection, notifier: Arc<dyn SchedulerNotifier>) -> Self {
        Self { db, notifier }
    }

    /// Load the active CA certificate PEM from the `ca_certificates` table.
    ///
    /// The active certificate is the most recently activated, non-deactivated entry.
    async fn load_active_ca_cert_pem(&self) -> Result<Option<String>, sea_orm::DbErr> {
        let cert = ca_certificate::Entity::find()
            .filter(ca_certificate::Column::DeactivatedAt.is_null())
            .order_by(ca_certificate::Column::ActivatedAt, Order::Desc)
            .one(&self.db)
            .await?;
        Ok(cert.map(|c| c.cert_pem))
    }
}

#[async_trait::async_trait]
impl TaskExecutor for ExternalCaRotationCheckExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> uptrakit_scheduler_engine::Result<()> {
        let cert_pem = self
            .load_active_ca_cert_pem()
            .await
            .context_to::<SchedulerError>()?;

        let Some(cert_pem) = cert_pem else {
            tracing::debug!("no active CA certificate found in database; skipping rotation check");
            return Ok(());
        };

        if ca_utils::should_rotate_ca(&cert_pem) {
            tracing::info!("CA certificate is within rotation window, requesting rotation via NATS");
            self.notifier
                .signal_ca_rotation(
                    "CA certificate approaching expiry (detected by external scheduler)",
                )
                .await;
        } else {
            tracing::debug!("CA certificate does not need rotation");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ActiveValue, ConnectOptions, ConnectionTrait, Database, Schema,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
    use uuid::Uuid;

    /// Minimal `SchedulerNotifier` that records whether `signal_ca_rotation` was called.
    struct TrackingNotifier(Arc<AtomicBool>);

    #[async_trait::async_trait]
    impl SchedulerNotifier for TrackingNotifier {
        async fn send_to_service(&self, _service_id: &Uuid, _msg: uptrakit_internal_wire::ControllerMessage) {}
        async fn broadcast(&self, _msg: uptrakit_internal_wire::ControllerMessage) {}
        async fn send_by_capability(&self, _capability: &str, _msg: uptrakit_internal_wire::ControllerMessage) {}
        async fn signal_ca_rotation(&self, _reason: &str) {
            self.0.store(true, Ordering::SeqCst);
        }
        async fn push_software_states_for_tenant(&self, _db: &sea_orm::DatabaseConnection, _tenant_id: Uuid) {}
    }

    fn dummy_task() -> scheduled_task::Model {
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

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        let schema = Schema::new(db.get_database_backend());
        let stmt = schema.create_table_from_entity(ca_certificate::Entity);
        db.execute(&stmt)
            .await
            .expect("create ca_certificates table");
        db
    }

    fn generate_test_ca_cert(days_valid: i64) -> (String, OffsetDateTime, OffsetDateTime) {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let not_before = OffsetDateTime::now_utc();
        let not_after = not_before + time::Duration::days(days_valid);
        params.not_before = not_before;
        params.not_after = not_after;
        let cert = params.self_signed(&key_pair).expect("self-sign");
        (cert.pem(), not_before, not_after)
    }

    #[tokio::test]
    async fn no_ca_cert_skips_rotation() {
        let db = setup_test_db().await;
        let triggered = Arc::new(AtomicBool::new(false));
        let notifier: Arc<dyn SchedulerNotifier> = Arc::new(TrackingNotifier(triggered.clone()));
        let executor = ExternalCaRotationCheckExecutor::new(db, notifier);

        executor.execute(&dummy_task()).await.expect("execute");
        assert!(
            !triggered.load(Ordering::SeqCst),
            "should not trigger when no CA cert exists"
        );
    }

    #[tokio::test]
    async fn long_lived_cert_does_not_trigger() {
        let db = setup_test_db().await;
        let (cert_pem, not_before, not_after) = generate_test_ca_cert(1825);

        // Insert the certificate using encrypted key (use a placeholder since we
        // don't need to actually encrypt for this test — the executor only reads cert_pem).
        ca_certificate::ActiveModel {
            fingerprint: ActiveValue::Set("test-fingerprint-long".to_string()),
            cert_pem: ActiveValue::Set(cert_pem),
            key_pem: ActiveValue::Set(uptrakit_shared_db::crypto::EncryptedString::new(
                "placeholder-key".to_string(),
            )
            .expect("no master key in test — stores plaintext")),
            not_before: ActiveValue::Set(not_before),
            not_after: ActiveValue::Set(not_after),
            activated_at: ActiveValue::Set(OffsetDateTime::now_utc()),
            deactivated_at: ActiveValue::Set(None),
            created_at: ActiveValue::Set(OffsetDateTime::now_utc()),
        }
        .insert(&db)
        .await
        .expect("insert cert");

        let triggered = Arc::new(AtomicBool::new(false));
        let notifier: Arc<dyn SchedulerNotifier> = Arc::new(TrackingNotifier(triggered.clone()));
        let executor = ExternalCaRotationCheckExecutor::new(db, notifier);

        executor.execute(&dummy_task()).await.expect("execute");
        assert!(
            !triggered.load(Ordering::SeqCst),
            "should not trigger for long-lived cert"
        );
    }

    #[tokio::test]
    async fn soon_expiring_cert_triggers() {
        let db = setup_test_db().await;
        let (cert_pem, not_before, not_after) = generate_test_ca_cert(30);

        ca_certificate::ActiveModel {
            fingerprint: ActiveValue::Set("test-fingerprint-short".to_string()),
            cert_pem: ActiveValue::Set(cert_pem),
            key_pem: ActiveValue::Set(uptrakit_shared_db::crypto::EncryptedString::new(
                "placeholder-key".to_string(),
            )
            .expect("no master key in test — stores plaintext")),
            not_before: ActiveValue::Set(not_before),
            not_after: ActiveValue::Set(not_after),
            activated_at: ActiveValue::Set(OffsetDateTime::now_utc()),
            deactivated_at: ActiveValue::Set(None),
            created_at: ActiveValue::Set(OffsetDateTime::now_utc()),
        }
        .insert(&db)
        .await
        .expect("insert cert");

        let triggered = Arc::new(AtomicBool::new(false));
        let notifier: Arc<dyn SchedulerNotifier> = Arc::new(TrackingNotifier(triggered.clone()));
        let executor = ExternalCaRotationCheckExecutor::new(db, notifier);

        executor.execute(&dummy_task()).await.expect("execute");
        assert!(
            triggered.load(Ordering::SeqCst),
            "should trigger for soon-expiring cert"
        );
    }
}
