use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{audit_log, scheduled_task, system_audit_log};

use crate::executor::TaskExecutor;

/// Default retention period for audit log entries (90 days).
const DEFAULT_RETENTION_DAYS: i64 = 90;

/// Deletes audit log entries older than the configured retention period.
///
/// Both `audit_logs` (tenant-scoped) and `system_audit_logs` (global) are
/// cleaned in a single database transaction to ensure atomicity.
///
/// Future: per-tenant retention overrides will be read from the
/// `audit_log.retention_days` setting key.
pub struct AuditLogCleanupExecutor {
    db: DatabaseConnection,
}

impl AuditLogCleanupExecutor {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for AuditLogCleanupExecutor {
    #[tracing::instrument(skip_all, fields(task = "audit_log_cleanup"))]
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::days(DEFAULT_RETENTION_DAYS);

        let txn = self.db.begin().await.context_to()?;

        let tenant_result = AuditLog::delete_many()
            .filter(audit_log::Column::OccurredAt.lt(cutoff))
            .exec(&txn)
            .await
            .context_to()?;

        let system_result = SystemAuditLog::delete_many()
            .filter(system_audit_log::Column::OccurredAt.lt(cutoff))
            .exec(&txn)
            .await
            .context_to()?;

        txn.commit().await.context_to()?;

        tracing::debug!(
            tenant_deleted = tenant_result.rows_affected,
            system_deleted = system_result.rows_affected,
            retention_days = DEFAULT_RETENTION_DAYS,
            "audit log cleanup completed"
        );

        Ok(())
    }
}
