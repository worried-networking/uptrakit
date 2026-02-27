use std::sync::Arc;

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_internal_wire::{ControllerMessage, RequestCertRenewalPayload};
use uptrakit_shared_db::entity::{scheduled_task, service_certificate};

use crate::error::SchedulerError;
use crate::executor::TaskExecutor;
use crate::notifier::SchedulerNotifier;

/// Default renewal window: request renewal when cert expires within this many days.
const RENEWAL_WINDOW_DAYS: i64 = 30;

/// Checks service certificates that are approaching expiry and sends renewal
/// requests to the owning services.
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
        let renewal_cutoff = now + time::Duration::days(RENEWAL_WINDOW_DAYS);

        // Find active (non-revoked) certs expiring within the renewal window.
        let expiring = service_certificate::Entity::find()
            .filter(service_certificate::Column::RevokedAt.is_null())
            .filter(service_certificate::Column::NotAfter.lte(renewal_cutoff))
            .filter(service_certificate::Column::NotAfter.gt(now))
            .all(&self.db)
            .await
            .context_to::<SchedulerError>()?;

        if expiring.is_empty() {
            tracing::debug!("no service certificates approaching renewal");
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
