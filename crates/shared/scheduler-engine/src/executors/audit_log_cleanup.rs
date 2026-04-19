use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};
use time::OffsetDateTime;
use uptrakit_audit_log::RuntimeAuditEmitter;
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
    audit_emitter: RuntimeAuditEmitter,
}

impl AuditLogCleanupExecutor {
    pub fn new(db: DatabaseConnection, audit_emitter: RuntimeAuditEmitter) -> Self {
        Self { db, audit_emitter }
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

        self.audit_emitter.scheduler_audit_log_cleanup(
            tenant_result.rows_affected,
            system_result.rows_affected,
            DEFAULT_RETENTION_DAYS,
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, EntityTrait, Set};
    use serde_json::json;
    use uptrakit_audit_log::{RuntimeAuditEmitter, RuntimeAuditEvent, RuntimeAuditForwarder};
    use uptrakit_shared_db::entity::{audit_log, scheduled_task, system_audit_log, tenant};
    use uptrakit_shared_db::migration::run_migrations;
    use uuid::Uuid;

    use super::*;

    #[derive(Default)]
    struct RecordingForwarder {
        events: Mutex<Vec<RuntimeAuditEvent>>,
    }

    impl RecordingForwarder {
        fn events(&self) -> Vec<RuntimeAuditEvent> {
            self.events.lock().expect("lock").clone()
        }
    }

    impl RuntimeAuditForwarder for RecordingForwarder {
        fn forward(&self, event: &RuntimeAuditEvent) {
            self.events.lock().expect("lock").push(event.clone());
        }
    }

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect(ConnectOptions::new("sqlite::memory:"))
            .await
            .expect("test db");
        run_migrations(&db).await.expect("run migrations");
        db
    }

    fn make_task() -> scheduled_task::Model {
        let now = OffsetDateTime::now_utc();
        scheduled_task::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            task_type: scheduled_task::ScheduledTaskType::AuditLogCleanup,
            interval_seconds: 86_400,
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

    async fn insert_audit_rows(db: &DatabaseConnection) -> (Uuid, Uuid) {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("tenant".to_string()),
            slug: Set(tenant_id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");

        let old_id = Uuid::now_v7();
        audit_log::ActiveModel {
            id: Set(old_id),
            tenant_id: Set(tenant_id),
            actor_id: Set(Some(Uuid::now_v7())),
            actor_type: Set("user".to_string()),
            actor_display: Set(Some("tester".to_string())),
            action_type: Set("plugin_config.create".to_string()),
            target_type: Set(Some("plugin_config".to_string())),
            target_id: Set(Some(Uuid::now_v7().to_string())),
            target_display: Set(Some("demo".to_string())),
            outcome: Set("success".to_string()),
            details_json: Set(Some(json!({ "old": true }))),
            request_id: Set(None),
            occurred_at: Set(now - time::Duration::days(100)),
        }
        .insert(db)
        .await
        .expect("insert tenant audit log");

        let recent_id = Uuid::now_v7();
        system_audit_log::ActiveModel {
            id: Set(recent_id),
            actor_id: Set(None),
            actor_type: Set("system".to_string()),
            actor_display: Set(Some("scheduler".to_string())),
            action_type: Set("system.scheduler.audit_log_cleanup".to_string()),
            target_type: Set(None),
            target_id: Set(None),
            target_display: Set(None),
            outcome: Set("success".to_string()),
            details_json: Set(Some(json!({ "recent": true }))),
            request_id: Set(None),
            occurred_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert system audit log");

        (old_id, recent_id)
    }

    #[tokio::test]
    async fn audit_log_cleanup_executor_writes_runtime_audit_event() {
        let db = setup_db().await;
        let (_old_id, recent_id) = insert_audit_rows(&db).await;
        let forwarder = Arc::new(RecordingForwarder::default());
        let executor = AuditLogCleanupExecutor::new(
            db.clone(),
            RuntimeAuditEmitter::with_forwarder(forwarder.clone()),
        );

        executor.execute(&make_task()).await.expect("execute");

        let tenant_rows = audit_log::Entity::find()
            .all(&db)
            .await
            .expect("tenant rows");
        let system_rows = system_audit_log::Entity::find()
            .all(&db)
            .await
            .expect("system rows");

        assert!(tenant_rows.is_empty());
        assert_eq!(system_rows.len(), 1);
        assert_eq!(system_rows[0].id, recent_id);

        let events = forwarder.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "system.scheduler.audit_log_cleanup");
        assert_eq!(events[0].details["tenant_deleted"], json!(1));
        assert_eq!(events[0].details["system_deleted"], json!(0));
        assert_eq!(events[0].details["retention_days"], json!(90));
    }
}
