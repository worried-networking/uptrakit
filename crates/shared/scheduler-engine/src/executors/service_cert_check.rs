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
