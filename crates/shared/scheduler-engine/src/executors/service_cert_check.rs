use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_internal_wire::{ControllerMessage, RequestCertRenewalPayload};
use uptrakit_shared_db::entity::{scheduled_task, service_certificate};

use crate::error::SchedulerError;
use crate::executor::TaskExecutor;
use crate::notifier::SchedulerNotifier;

/// Maximum renewal window (ceiling), regardless of cert lifetime.
const MAX_RENEWAL_WINDOW_DAYS: i64 = 14;

/// Compute the renewal window for a single certificate.
///
/// The window is `min(MAX_RENEWAL_WINDOW_DAYS, cert_lifetime / 5)` where
/// `cert_lifetime` is the duration between `not_before` and `not_after`.
/// Returns [`time::Duration::ZERO`] for certs with zero or negative lifetime.
pub fn cert_renewal_window(
    not_before: OffsetDateTime,
    not_after: OffsetDateTime,
) -> time::Duration {
    let lifetime_hours = (not_after - not_before).whole_hours().max(0);
    let window_hours = (lifetime_hours / 5).min(MAX_RENEWAL_WINDOW_DAYS * 24);
    time::Duration::hours(window_hours)
}

/// Checks service certificates that are approaching expiry and sends renewal
/// requests to the owning services.
///
/// The renewal window for each certificate is `min(14 days, cert_lifetime / 5)`,
/// making it proportional to the certificate's own TTL with a 14-day ceiling.
pub struct ServiceCertCheckExecutor {
    db: DatabaseConnection,
    notifier: Arc<dyn SchedulerNotifier>,
}

impl ServiceCertCheckExecutor {
    pub fn new(db: DatabaseConnection, notifier: Arc<dyn SchedulerNotifier>) -> Self {
        Self { db, notifier }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for ServiceCertCheckExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
        let now = OffsetDateTime::now_utc();

        // The maximum possible renewal window is MAX_RENEWAL_WINDOW_DAYS.
        // Only certs expiring within that window can possibly need renewal.
        let max_cutoff = now + time::Duration::days(MAX_RENEWAL_WINDOW_DAYS);

        // Load non-revoked, non-expired certs within the maximum window.
        // Per-cert filtering in Rust narrows this to the individual TTL-derived window.
        let candidates = service_certificate::Entity::find()
            .filter(service_certificate::Column::RevokedAt.is_null())
            .filter(service_certificate::Column::NotAfter.gt(now))
            .filter(service_certificate::Column::NotAfter.lte(max_cutoff))
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        if candidates.is_empty() {
            tracing::debug!("no service certificates approaching renewal");
            return Ok(());
        }

        // Filter per-cert using the individual TTL-derived renewal window.
        let expiring: Vec<_> = candidates
            .iter()
            .filter(|cert| {
                let window = cert_renewal_window(cert.not_before, cert.not_after);
                cert.not_after <= now + window
            })
            .collect();

        if expiring.is_empty() {
            tracing::debug!("no service certificates within their TTL renewal window");
            return Ok(());
        }

        let mut sent = 0usize;
        for cert in &expiring {
            let days_left = (cert.not_after - now).whole_days();
            let msg = ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload {
                reason: format!(
                    "certificate expires in {days_left} day{} (proactive renewal)",
                    if days_left == 1 { "" } else { "s" }
                ),
            });
            self.notifier.send_to_service(&cert.service_id, msg).await;
            sent += 1;
        }

        tracing::debug!(
            expiring = expiring.len(),
            sent,
            "sent service certificate renewal requests"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, Set};
    use uptrakit_crypto::EncryptedString;
    use uptrakit_shared_db::entity::{ca_certificate, service, service_certificate, tenant};
    use uptrakit_shared_db::migration::run_migrations;
    use uuid::Uuid;

    async fn setup_db() -> sea_orm::DatabaseConnection {
        // Enable plaintext crypto mode so EncryptedString columns (e.g.
        // ca_certificate.key_pem) can be written without a real key ring.
        uptrakit_crypto::enable_plaintext_mode();
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        run_migrations(&db).await.expect("run migrations");
        db
    }

    /// Insert all rows required to satisfy FK constraints for
    /// `service_certificates`: tenant → ca_certificate + service.
    /// Returns `(ca_fingerprint, service_id)`.
    async fn insert_service_for_test(db: &sea_orm::DatabaseConnection) -> (String, Uuid) {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set(tenant_id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");

        let fingerprint = format!("fp-{}", Uuid::now_v7());
        ca_certificate::ActiveModel {
            fingerprint: Set(fingerprint.clone()),
            cert_pem: Set("---TEST CERT---".to_string()),
            key_pem: Set(EncryptedString::plaintext_for_test(
                "---TEST KEY---".to_string(),
            )),
            not_before: Set(now - time::Duration::days(1)),
            not_after: Set(now + time::Duration::days(365)),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert ca_certificate");

        let service_id = Uuid::now_v7();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("host-{service_id}")),
            friendly_name: Set("Test Service".to_string()),
            ip_address: Set(None),
            status: Set(uptrakit_shared_types::ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
        }
        .insert(db)
        .await
        .expect("insert service");

        (fingerprint, service_id)
    }

    fn make_task() -> scheduled_task::Model {
        let now = OffsetDateTime::now_utc();
        scheduled_task::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            task_type:
                uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType::ServiceCertCheck,
            interval_seconds: 3600,
            jitter_seconds: 0,
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: now,
            locked_by: None,
            locked_at: None,
            last_error: None,
            run_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// A spy notifier that records each `send_to_service` call.
    struct SpyNotifier {
        sent: std::sync::Mutex<Vec<Uuid>>,
    }

    impl SpyNotifier {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                sent: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn sent_count(&self) -> usize {
            self.sent.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl SchedulerNotifier for SpyNotifier {
        async fn send_to_service(&self, service_id: &Uuid, _msg: ControllerMessage) {
            self.sent.lock().unwrap().push(*service_id);
        }
        async fn broadcast(&self, _msg: ControllerMessage) {}
        async fn send_by_capability(&self, _cap: &str, _msg: ControllerMessage) {}
        async fn signal_ca_rotation(&self, _reason: &str) {}
        async fn signal_software_states_changed(&self, _tenant_id: uuid::Uuid) {}
        async fn signal_crl_renewal(&self) {}
    }

    // ── execute() tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_empty_db_returns_ok() {
        let db = setup_db().await;
        let notifier = SpyNotifier::new();
        let executor =
            ServiceCertCheckExecutor::new(db, notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor
            .execute(&make_task())
            .await
            .expect("should succeed");
        assert_eq!(notifier.sent_count(), 0);
    }

    #[tokio::test]
    async fn execute_non_expiring_cert_sends_no_message() {
        let db = setup_db().await;
        let now = OffsetDateTime::now_utc();
        // Cert valid for 365 days, not expiring soon.
        let not_before = now - time::Duration::days(10);
        let not_after = now + time::Duration::days(355);
        let (ca_fingerprint, service_id) = insert_service_for_test(&db).await;

        service_certificate::ActiveModel {
            ca_fingerprint: Set(ca_fingerprint),
            serial_number: Set("sn-01".to_string()),
            service_id: Set(service_id),
            not_before: Set(not_before),
            not_after: Set(not_after),
            revoked_at: Set(None),
            revocation_reason: Set(None),
            created_at: Set(now),
            last_seen_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert cert");

        let notifier = SpyNotifier::new();
        let executor =
            ServiceCertCheckExecutor::new(db, notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor
            .execute(&make_task())
            .await
            .expect("should succeed");
        assert_eq!(
            notifier.sent_count(),
            0,
            "long-lived cert should not trigger renewal"
        );
    }

    #[tokio::test]
    async fn execute_expiring_cert_sends_renewal_message() {
        let db = setup_db().await;
        let now = OffsetDateTime::now_utc();
        // 7-day cert → renewal window = 33 h.
        // Set not_after to now + 20 h (within the 33 h window).
        let not_before = now - time::Duration::days(6);
        let not_after = now + time::Duration::hours(20);
        let (ca_fingerprint, service_id) = insert_service_for_test(&db).await;

        service_certificate::ActiveModel {
            ca_fingerprint: Set(ca_fingerprint),
            serial_number: Set("sn-02".to_string()),
            service_id: Set(service_id),
            not_before: Set(not_before),
            not_after: Set(not_after),
            revoked_at: Set(None),
            revocation_reason: Set(None),
            created_at: Set(now),
            last_seen_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert cert");

        let notifier = SpyNotifier::new();
        let executor =
            ServiceCertCheckExecutor::new(db, notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor
            .execute(&make_task())
            .await
            .expect("should succeed");
        assert_eq!(
            notifier.sent_count(),
            1,
            "expiring cert should trigger one renewal message"
        );
        assert_eq!(notifier.sent.lock().unwrap()[0], service_id);
    }

    #[tokio::test]
    async fn execute_revoked_cert_sends_no_message() {
        let db = setup_db().await;
        let now = OffsetDateTime::now_utc();
        let not_before = now - time::Duration::days(6);
        let not_after = now + time::Duration::hours(20);
        let (ca_fingerprint, service_id) = insert_service_for_test(&db).await;

        service_certificate::ActiveModel {
            ca_fingerprint: Set(ca_fingerprint),
            serial_number: Set("sn-03".to_string()),
            service_id: Set(service_id),
            not_before: Set(not_before),
            not_after: Set(not_after),
            revoked_at: Set(Some(now - time::Duration::hours(1))),
            revocation_reason: Set(Some(
                service_certificate::RevocationReason::CertificateRenewed,
            )),
            created_at: Set(now),
            last_seen_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert revoked cert");

        let notifier = SpyNotifier::new();
        let executor =
            ServiceCertCheckExecutor::new(db, notifier.clone() as Arc<dyn SchedulerNotifier>);
        executor
            .execute(&make_task())
            .await
            .expect("should succeed");
        assert_eq!(
            notifier.sent_count(),
            0,
            "revoked cert must not trigger renewal"
        );
    }

    // ── cert_renewal_window ─────────────────────────────────────────────────

    #[test]
    fn renewal_window_uses_one_fifth_of_lifetime_short_cert() {
        let base = OffsetDateTime::UNIX_EPOCH;
        // 7-day cert → 168 / 5 = 33 h (well under the 14-day = 336 h ceiling)
        let not_after = base + time::Duration::days(7);
        let window = cert_renewal_window(base, not_after);
        assert_eq!(window.whole_hours(), 33);
    }

    #[test]
    fn renewal_window_applies_ceiling_for_long_cert() {
        let base = OffsetDateTime::UNIX_EPOCH;
        // 365-day cert → 365*24/5 = 1752 h, ceiling 336 h (14 days) kicks in
        let not_after = base + time::Duration::days(365);
        let window = cert_renewal_window(base, not_after);
        assert_eq!(window.whole_hours(), MAX_RENEWAL_WINDOW_DAYS * 24);
    }

    #[test]
    fn renewal_window_ceiling_at_exactly_70_day_cert() {
        let base = OffsetDateTime::UNIX_EPOCH;
        // 70-day cert → 70*24/5 = 336 h = exactly 14 days (ceiling boundary)
        let not_after = base + time::Duration::days(70);
        let window = cert_renewal_window(base, not_after);
        assert_eq!(window.whole_hours(), MAX_RENEWAL_WINDOW_DAYS * 24);
    }

    #[test]
    fn renewal_window_zero_lifetime_returns_zero() {
        let base = OffsetDateTime::UNIX_EPOCH;
        // Zero lifetime → 0 / 5 = 0 → min(0, 336) = 0
        let window = cert_renewal_window(base, base);
        assert_eq!(window.whole_hours(), 0);
    }

    #[test]
    fn renewal_window_1_day_cert() {
        let base = OffsetDateTime::UNIX_EPOCH;
        // 1-day cert → 24 / 5 = 4 h
        let not_after = base + time::Duration::days(1);
        let window = cert_renewal_window(base, not_after);
        assert_eq!(window.whole_hours(), 4);
    }
}
