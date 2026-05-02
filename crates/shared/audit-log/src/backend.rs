use std::sync::Arc;

use crate::entry::AuditEntry;
use crate::error::AuditLogError;

/// Trait for audit log storage backends.
#[async_trait::async_trait]
pub trait AuditLogBackend: Send + Sync {
    /// Persist a single audit entry. Implementations must not panic.
    async fn write(&self, entry: &AuditEntry) -> std::result::Result<(), AuditLogError>;
}

/// Backend that silently discards all entries. Used when audit logging is disabled.
pub struct NoopBackend;

#[async_trait::async_trait]
impl AuditLogBackend for NoopBackend {
    async fn write(&self, _entry: &AuditEntry) -> std::result::Result<(), AuditLogError> {
        Ok(())
    }
}

/// Backend that fans out to multiple backends concurrently.
///
/// Errors from individual backends are logged but do not affect other backends.
pub struct MultiplexBackend {
    backends: Vec<Arc<dyn AuditLogBackend>>,
}

impl MultiplexBackend {
    pub fn new(backends: Vec<Arc<dyn AuditLogBackend>>) -> Self {
        Self { backends }
    }
}

#[async_trait::async_trait]
impl AuditLogBackend for MultiplexBackend {
    async fn write(&self, entry: &AuditEntry) -> std::result::Result<(), AuditLogError> {
        let futures: Vec<_> = self
            .backends
            .iter()
            .map(|backend| {
                let backend = Arc::clone(backend);
                let entry = entry.clone();
                async move {
                    if let Err(e) = backend.write(&entry).await {
                        tracing::warn!(error = %e, "audit log backend write failed");
                    }
                }
            })
            .collect();

        futures_util::future::join_all(futures).await;
        Ok(())
    }
}

/// Database backend that persists entries to `audit_logs` / `system_audit_logs` tables.
#[cfg(feature = "db")]
pub struct DatabaseBackend {
    db: sea_orm::DatabaseConnection,
}

#[cfg(feature = "db")]
impl DatabaseBackend {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }
}

#[cfg(feature = "db")]
#[async_trait::async_trait]
impl AuditLogBackend for DatabaseBackend {
    async fn write(&self, entry: &AuditEntry) -> std::result::Result<(), AuditLogError> {
        use sea_orm::{ActiveValue::Set, EntityTrait};

        if let Err(err) = entry.validate() {
            return Err(AuditLogError::Validation(err.to_string()));
        }

        if let Some(tenant_id) = entry.tenant_id {
            // Tenant-scoped audit log
            let model = uptrakit_shared_db::entity::audit_log::ActiveModel {
                id: Set(entry.id),
                tenant_id: Set(tenant_id),
                actor_id: Set(entry.actor_id),
                actor_type: Set(entry.actor_type.as_str().to_string()),
                actor_display: Set(entry.actor_display.clone()),
                action_type: Set(entry.action_type.as_str().to_string()),
                target_type: Set(entry.target_type.clone()),
                target_id: Set(entry.target_id.clone()),
                target_display: Set(entry.target_display.clone()),
                outcome: Set(entry.outcome.as_str().to_string()),
                details_json: Set(entry.details_json.clone()),
                request_id: Set(entry.request_id.clone()),
                occurred_at: Set(entry.occurred_at),
            };

            uptrakit_shared_db::entity::audit_log::Entity::insert(model)
                .exec(&self.db)
                .await?;
        } else {
            // System-level audit log
            let model = uptrakit_shared_db::entity::system_audit_log::ActiveModel {
                id: Set(entry.id),
                actor_id: Set(entry.actor_id),
                actor_type: Set(entry.actor_type.as_str().to_string()),
                actor_display: Set(entry.actor_display.clone()),
                action_type: Set(entry.action_type.as_str().to_string()),
                target_type: Set(entry.target_type.clone()),
                target_id: Set(entry.target_id.clone()),
                target_display: Set(entry.target_display.clone()),
                outcome: Set(entry.outcome.as_str().to_string()),
                details_json: Set(entry.details_json.clone()),
                request_id: Set(entry.request_id.clone()),
                occurred_at: Set(entry.occurred_at),
            };

            uptrakit_shared_db::entity::system_audit_log::Entity::insert(model)
                .exec(&self.db)
                .await?;
        }

        Ok(())
    }
}

/// Journald backend that emits structured tracing events.
///
/// Requires the `journald` Cargo feature and a running journald instance.
/// The tracing layer must be configured with a journald subscriber for these
/// events to actually reach the journal.
#[cfg(feature = "journald")]
pub struct JournaldBackend;

#[cfg(feature = "journald")]
#[async_trait::async_trait]
impl AuditLogBackend for JournaldBackend {
    async fn write(&self, entry: &AuditEntry) -> std::result::Result<(), AuditLogError> {
        if let Err(err) = entry.validate() {
            return Err(AuditLogError::Validation(err.to_string()));
        }

        tracing::info!(
            target: "uptrakit_audit",
            audit_id = %entry.id,
            tenant_id = ?entry.tenant_id,
            actor_id = ?entry.actor_id,
            actor_type = %entry.actor_type,
            actor_display = ?entry.actor_display,
            action_type = %entry.action_type,
            target_type = ?entry.target_type,
            target_id = ?entry.target_id,
            target_display = ?entry.target_display,
            outcome = %entry.outcome,
            details_json = ?entry.details_json,
            request_id = ?entry.request_id,
            occurred_at = %entry.occurred_at,
            "audit log entry"
        );
        Ok(())
    }
}

#[cfg(all(test, feature = "db"))]
mod db_tests {
    #![expect(
        clippy::expect_used,
        reason = "test helpers — expect is used in setup functions and mock implementations; panic is acceptable in test context"
    )]

    use super::*;
    use sea_orm::{
        ConnectOptions, ConnectionTrait as _, Database, DatabaseConnection, EntityTrait as _,
    };
    use uuid::Uuid;

    use crate::AuditActionType;

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect(ConnectOptions::new("sqlite::memory:"))
            .await
            .expect("test db should open");

        db.execute_unprepared(
            r#"
            CREATE TABLE audit_logs (
                id BLOB PRIMARY KEY NOT NULL,
                tenant_id BLOB NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id BLOB NULL,
                actor_display TEXT NULL,
                action_type TEXT NOT NULL,
                target_type TEXT NULL,
                target_id TEXT NULL,
                target_display TEXT NULL,
                outcome TEXT NOT NULL,
                details_json TEXT NULL,
                request_id TEXT NULL,
                occurred_at TEXT NOT NULL
            )
            "#,
        )
        .await
        .expect("tenant audit table should be created");

        db.execute_unprepared(
            r#"
            CREATE TABLE system_audit_logs (
                id BLOB PRIMARY KEY NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id BLOB NULL,
                actor_display TEXT NULL,
                action_type TEXT NOT NULL,
                target_type TEXT NULL,
                target_id TEXT NULL,
                target_display TEXT NULL,
                outcome TEXT NOT NULL,
                details_json TEXT NULL,
                request_id TEXT NULL,
                occurred_at TEXT NOT NULL
            )
            "#,
        )
        .await
        .expect("system audit table should be created");

        db
    }

    #[tokio::test]
    async fn database_backend_persists_semantic_audit_entry() {
        let db = setup_db().await;
        let backend = DatabaseBackend::new(db.clone());
        let entry = AuditEntry::builder(AuditActionType::PLUGIN_CONFIG_CREATE)
            .tenant_scope(Uuid::now_v7())
            .build()
            .expect("stub entry should validate");

        backend.write(&entry).await.expect("insert should succeed");
        let row = uptrakit_shared_db::entity::audit_log::Entity::find()
            .one(&db)
            .await
            .expect("query should succeed")
            .expect("row should exist");

        assert_eq!(row.action_type, "plugin_config.create");
    }

    #[tokio::test]
    async fn database_backend_routes_system_scope_entries_to_system_table() {
        let db = setup_db().await;
        let backend = DatabaseBackend::new(db.clone());
        let entry = AuditEntry::builder(AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP)
            .system_scope()
            .build()
            .expect("stub entry should validate");

        backend.write(&entry).await.expect("insert should succeed");
        let row = uptrakit_shared_db::entity::system_audit_log::Entity::find()
            .one(&db)
            .await
            .expect("query should succeed")
            .expect("row should exist");

        assert_eq!(row.action_type, "system.scheduler.audit_log_cleanup");
    }
}

#[cfg(all(test, feature = "journald"))]
mod journald_tests {
    #![expect(
        clippy::expect_used,
        reason = "test helper — Mutex lock is expected to succeed; panic on poisoned lock is acceptable in test context"
    )]

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::subscriber::Interest;
    use tracing::{Event, Metadata, Subscriber};

    use crate::AuditActionType;

    #[derive(Default)]
    struct FieldCapture {
        observed: Arc<Mutex<Vec<HashMap<String, String>>>>,
    }

    #[derive(Default)]
    struct FieldValueVisitor {
        values: HashMap<String, String>,
    }

    impl Visit for FieldValueVisitor {
        fn record_debug(&mut self, field: &Field, _value: &dyn std::fmt::Debug) {
            self.values
                .insert(field.name().to_string(), format!("{_value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.values
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.values
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.values
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.values
                .insert(field.name().to_string(), value.to_string());
        }
    }

    impl Subscriber for FieldCapture {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = FieldValueVisitor::default();
            event.record(&mut visitor);
            self.observed
                .lock()
                .expect("capture lock should not be poisoned")
                .push(visitor.values);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}

        fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
            Interest::always()
        }

        fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
            Some(tracing::level_filters::LevelFilter::TRACE)
        }

        fn clone_span(&self, id: &Id) -> Id {
            id.clone()
        }

        fn try_close(&self, _id: Id) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn journald_backend_emits_semantic_field_contract() {
        let capture = FieldCapture::default();
        let observed = Arc::clone(&capture.observed);
        let _guard = tracing::subscriber::set_default(capture);

        let backend = JournaldBackend;
        let entry = AuditEntry::builder(AuditActionType::PLUGIN_CONFIG_CREATE)
            .build()
            .expect("stub entry should validate");
        backend.write(&entry).await.expect("write should succeed");

        let events = observed
            .lock()
            .expect("capture lock should not be poisoned");
        let record = events.last().expect("expected one tracing event");

        assert!(record.contains_key("audit_id"));
        assert_eq!(
            record.get("action_type").map(String::as_str),
            Some("plugin_config.create")
        );
        assert_eq!(record.get("outcome").map(String::as_str), Some("success"));
        assert_eq!(record.get("tenant_id").map(String::as_str), Some("None"));
        assert!(record.contains_key("details_json"));
    }
}
