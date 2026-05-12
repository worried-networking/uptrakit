use std::sync::Arc;

use crate::entry::AuditEntryErased;
#[cfg(any(feature = "db", feature = "journald"))]
use crate::error::AuditLogError;
use crate::error::Result;

/// Trait for audit log storage backends.
#[async_trait::async_trait]
pub trait AuditLogBackend: Send + Sync {
    /// Persist a single audit entry. Implementations must not panic.
    ///
    /// # Errors
    ///
    /// Returns [`AuditLogError`] (wrapped in `rootcause::Report`) when the
    /// underlying storage write fails.
    async fn write(&self, entry: &AuditEntryErased) -> Result<()>;

    /// Write within a caller-supplied sea-orm transaction.
    ///
    /// DB backends override this to execute the INSERT on `tx` so the
    /// audit write participates in the caller's transaction. The default
    /// implementation calls [`write`][Self::write] and ignores `_tx`.
    ///
    /// Only available when the `db` feature is enabled.
    ///
    /// # Errors
    ///
    /// Returns [`AuditLogError`] (wrapped in `rootcause::Report`) when the
    /// underlying storage write fails.
    #[cfg(feature = "db")]
    async fn write_in_tx(
        &self,
        entry: &AuditEntryErased,
        _tx: &sea_orm::DatabaseTransaction,
    ) -> Result<()> {
        self.write(entry).await
    }
}

/// Backend that silently discards all entries. Used when audit logging is disabled.
pub struct NoopBackend;

#[async_trait::async_trait]
impl AuditLogBackend for NoopBackend {
    async fn write(&self, _entry: &AuditEntryErased) -> Result<()> {
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
    /// Creates a new multiplex backend that fans out writes to all supplied backends.
    pub fn new(backends: Vec<Arc<dyn AuditLogBackend>>) -> Self {
        Self { backends }
    }
}

#[async_trait::async_trait]
impl AuditLogBackend for MultiplexBackend {
    async fn write(&self, entry: &AuditEntryErased) -> Result<()> {
        let futures: Vec<_> = self
            .backends
            .iter()
            .map(|backend| {
                let backend = Arc::clone(backend);
                let entry = entry.clone();
                async move {
                    if let Err(e) = backend.write(&entry).await {
                        tracing::error!(error = %e, "audit log backend write failed");
                    }
                }
            })
            .collect();

        futures_util::future::join_all(futures).await;
        Ok(())
    }

    /// Delegates `write_in_tx` to the first backend (typically the DB backend).
    ///
    /// All other backends receive a regular `write` call so that non-DB backends
    /// (e.g. journald) still emit the entry even though they cannot participate
    /// in the transaction.
    #[cfg(feature = "db")]
    async fn write_in_tx(
        &self,
        entry: &AuditEntryErased,
        tx: &sea_orm::DatabaseTransaction,
    ) -> Result<()> {
        let mut iter = self.backends.iter();
        if let Some(first) = iter.next() {
            first.write_in_tx(entry, tx).await?;
        }
        for backend in iter {
            if let Err(e) = backend.write(entry).await {
                tracing::error!(error = %e, "audit log backend write failed");
            }
        }
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
    /// Creates a new database backend wrapping the supplied connection.
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }
}

#[cfg(feature = "db")]
#[async_trait::async_trait]
impl AuditLogBackend for DatabaseBackend {
    async fn write(&self, entry: &AuditEntryErased) -> Result<()> {
        db_insert(entry, &self.db).await
    }

    async fn write_in_tx(
        &self,
        entry: &AuditEntryErased,
        tx: &sea_orm::DatabaseTransaction,
    ) -> Result<()> {
        db_insert(entry, tx).await
    }
}

/// Shared INSERT logic that works against any sea-orm `ConnectionTrait` (pool
/// connection or transaction).
#[cfg(feature = "db")]
async fn db_insert(entry: &AuditEntryErased, conn: &impl sea_orm::ConnectionTrait) -> Result<()> {
    use sea_orm::{ActiveValue::Set, EntityTrait};

    use crate::entry::validate_erased;

    validate_erased(entry)?;

    if let Some(tenant_id) = entry.tenant_id {
        let model = uptrakit_shared_db::entity::audit_log::ActiveModel {
            id: Set(entry.id),
            tenant_id: Set(tenant_id),
            actor_id: Set(entry.actor_id),
            actor_type: Set(entry.actor_type.as_str().to_string()),
            actor_display: Set(entry.actor_display.clone()),
            action_type: Set(entry.action_type.as_str().to_string()),
            action_kind: Set(entry.action_kind.as_str().to_string()),
            target_type: Set(entry.target_type.clone()),
            target_id: Set(entry.target_id.clone()),
            target_display: Set(entry.target_display.clone()),
            outcome: Set(entry.outcome.as_str().to_string()),
            details_json: Set(entry.details_json.clone()),
            before_snapshot: Set(entry.before_snapshot.clone()),
            after_snapshot: Set(entry.after_snapshot.clone()),
            correlation_id: Set(entry.correlation_id),
            request_id: Set(entry.request_id.clone()),
            occurred_at: Set(entry.occurred_at),
        };

        uptrakit_shared_db::entity::audit_log::Entity::insert(model)
            .exec(conn)
            .await
            .map_err(AuditLogError::Database)
            .map_err(rootcause::Report::from)?;
    } else {
        let model = uptrakit_shared_db::entity::system_audit_log::ActiveModel {
            id: Set(entry.id),
            actor_id: Set(entry.actor_id),
            actor_type: Set(entry.actor_type.as_str().to_string()),
            actor_display: Set(entry.actor_display.clone()),
            action_type: Set(entry.action_type.as_str().to_string()),
            action_kind: Set(entry.action_kind.as_str().to_string()),
            target_type: Set(entry.target_type.clone()),
            target_id: Set(entry.target_id.clone()),
            target_display: Set(entry.target_display.clone()),
            outcome: Set(entry.outcome.as_str().to_string()),
            details_json: Set(entry.details_json.clone()),
            before_snapshot: Set(entry.before_snapshot.clone()),
            after_snapshot: Set(entry.after_snapshot.clone()),
            correlation_id: Set(entry.correlation_id),
            request_id: Set(entry.request_id.clone()),
            occurred_at: Set(entry.occurred_at),
        };

        uptrakit_shared_db::entity::system_audit_log::Entity::insert(model)
            .exec(conn)
            .await
            .map_err(AuditLogError::Database)
            .map_err(rootcause::Report::from)?;
    }

    Ok(())
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
    async fn write(&self, entry: &AuditEntryErased) -> Result<()> {
        use crate::entry::validate_erased;

        validate_erased(entry)?;

        tracing::info!(
            target: "uptrakit_audit",
            audit_id = %entry.id,
            tenant_id = ?entry.tenant_id,
            actor_id = ?entry.actor_id,
            actor_type = %entry.actor_type,
            actor_display = ?entry.actor_display,
            action_type = %entry.action_type,
            action_kind = %entry.action_kind,
            target_type = ?entry.target_type,
            target_id = ?entry.target_id,
            target_display = ?entry.target_display,
            outcome = %entry.outcome,
            details_json = ?entry.details_json,
            correlation_id = ?entry.correlation_id,
            request_id = ?entry.request_id,
            occurred_at = %entry.occurred_at,
            before_snapshot_bytes = entry.before_snapshot.as_ref().map_or(0, |v| serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0)),
            after_snapshot_bytes = entry.after_snapshot.as_ref().map_or(0, |v| serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0)),
            "audit"
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
    use crate::entry::{AuditActorType, AuditEntry, Event};

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
                action_kind TEXT NOT NULL,
                target_type TEXT NULL,
                target_id TEXT NULL,
                target_display TEXT NULL,
                outcome TEXT NOT NULL,
                details_json TEXT NULL,
                before_snapshot TEXT NULL,
                after_snapshot TEXT NULL,
                correlation_id BLOB NULL,
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
                action_kind TEXT NOT NULL,
                target_type TEXT NULL,
                target_id TEXT NULL,
                target_display TEXT NULL,
                outcome TEXT NOT NULL,
                details_json TEXT NULL,
                before_snapshot TEXT NULL,
                after_snapshot TEXT NULL,
                correlation_id BLOB NULL,
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
        let entry: AuditEntryErased =
            AuditEntry::<Event>::builder_event(AuditActionType::PLUGIN_CONFIG_CREATE)
                .tenant_scope(Uuid::now_v7())
                .build()
                .expect("stub entry should validate")
                .into();

        backend.write(&entry).await.expect("insert should succeed");
        let row = uptrakit_shared_db::entity::audit_log::Entity::find()
            .one(&db)
            .await
            .expect("query should succeed")
            .expect("row should exist");

        assert_eq!(row.action_type, "plugin_config.create");
        assert_eq!(row.action_kind, "event");
    }

    #[tokio::test]
    async fn database_backend_routes_system_scope_entries_to_system_table() {
        let db = setup_db().await;
        let backend = DatabaseBackend::new(db.clone());
        let entry: AuditEntryErased =
            AuditEntry::<Event>::builder_event(AuditActionType::SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP)
                .system_scope()
                .build()
                .expect("stub entry should validate")
                .into();

        backend.write(&entry).await.expect("insert should succeed");
        let row = uptrakit_shared_db::entity::system_audit_log::Entity::find()
            .one(&db)
            .await
            .expect("query should succeed")
            .expect("row should exist");

        assert_eq!(row.action_type, "system.scheduler.audit_log_cleanup");
        assert_eq!(row.action_kind, "event");
    }

    #[tokio::test]
    async fn database_backend_persists_correlation_id() {
        let db = setup_db().await;
        let backend = DatabaseBackend::new(db.clone());
        let correlation = Uuid::now_v7();
        let entry: AuditEntryErased =
            AuditEntry::<Event>::builder_event(AuditActionType::AUTH_LOGIN)
                .tenant_scope(Uuid::now_v7())
                .correlation_id(correlation)
                .build()
                .expect("stub entry should validate")
                .into();

        backend.write(&entry).await.expect("insert should succeed");
        let row = uptrakit_shared_db::entity::audit_log::Entity::find()
            .one(&db)
            .await
            .expect("query should succeed")
            .expect("row should exist");

        assert_eq!(row.correlation_id, Some(correlation));
    }

    #[tokio::test]
    async fn database_backend_write_in_tx_persists_entry() {
        use sea_orm::TransactionTrait as _;

        let db = setup_db().await;
        let backend = DatabaseBackend::new(db.clone());
        let entry: AuditEntryErased =
            AuditEntry::<Event>::builder_event(AuditActionType::AUTH_LOGOUT)
                .tenant_scope(Uuid::now_v7())
                .build()
                .expect("stub entry should validate")
                .into();

        let tx = db.begin().await.expect("txn should start");
        backend
            .write_in_tx(&entry, &tx)
            .await
            .expect("write_in_tx should succeed");
        tx.commit().await.expect("txn should commit");

        let row = uptrakit_shared_db::entity::audit_log::Entity::find()
            .one(&db)
            .await
            .expect("query should succeed")
            .expect("row should exist after commit");

        assert_eq!(row.action_type, "auth.logout");
    }

    #[tokio::test]
    #[cfg(feature = "db-sqlite")]
    async fn write_in_tx_rollback_leaves_no_row() {
        use sea_orm::TransactionTrait as _;

        let db = setup_db().await;
        let entry: AuditEntryErased =
            AuditEntry::<Event>::builder_event(AuditActionType::AUTH_LOGOUT)
                .tenant_scope(Uuid::now_v7())
                .build()
                .expect("stub entry should validate")
                .into();

        let tx = db.begin().await.expect("txn should start");
        DatabaseBackend::new(db.clone())
            .write_in_tx(&entry, &tx)
            .await
            .expect("write_in_tx should succeed");
        tx.rollback().await.expect("rollback should succeed");

        let row = uptrakit_shared_db::entity::audit_log::Entity::find()
            .one(&db)
            .await
            .expect("query should succeed");
        assert!(row.is_none(), "rollback should leave no audit row");
    }

    #[tokio::test]
    async fn noop_backend_accepts_erased_entries() {
        let entry: AuditEntryErased =
            AuditEntry::<Event>::builder_event(AuditActionType::AUTH_LOGIN)
                .actor(AuditActorType::System, None)
                .build()
                .expect("entry should validate")
                .into();
        NoopBackend
            .write(&entry)
            .await
            .expect("noop should not fail");
    }
}

#[cfg(all(test, feature = "journald"))]
mod journald_tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use parking_lot::Mutex;

    use super::*;
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::subscriber::Interest;
    use tracing::{Event as TracingEvent, Metadata, Subscriber};

    use crate::AuditActionType;
    use crate::entry::{AuditEntry, Event};

    #[derive(Default)]
    struct FieldCapture {
        observed: Arc<Mutex<Vec<HashMap<String, String>>>>,
    }

    #[derive(Default)]
    struct FieldValueVisitor {
        values: HashMap<String, String>,
    }

    impl Visit for FieldValueVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.values
                .insert(field.name().to_string(), format!("{value:?}"));
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

        fn event(&self, event: &TracingEvent<'_>) {
            let mut visitor = FieldValueVisitor::default();
            event.record(&mut visitor);
            self.observed.lock().push(visitor.values);
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
        let entry: AuditEntryErased =
            AuditEntry::<Event>::builder_event(AuditActionType::PLUGIN_CONFIG_CREATE)
                .build()
                .expect("stub entry should validate")
                .into();
        backend.write(&entry).await.expect("write should succeed");

        let events = observed.lock();
        let record = events.last().expect("expected one tracing event");

        assert!(record.contains_key("audit_id"));
        assert_eq!(
            record.get("action_type").map(String::as_str),
            Some("plugin_config.create")
        );
        assert_eq!(record.get("action_kind").map(String::as_str), Some("event"));
        assert_eq!(record.get("outcome").map(String::as_str), Some("success"));
        assert_eq!(record.get("tenant_id").map(String::as_str), Some("None"));
        assert!(record.contains_key("details_json"));
        assert!(record.contains_key("before_snapshot_bytes"));
        assert!(record.contains_key("after_snapshot_bytes"));
    }
}
