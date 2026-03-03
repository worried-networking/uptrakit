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

        if let Some(tenant_id) = entry.tenant_id {
            // Tenant-scoped audit log
            let model = uptrakit_shared_db::entity::audit_log::ActiveModel {
                id: Set(entry.id),
                tenant_id: Set(tenant_id),
                actor_id: Set(entry.actor_id),
                actor_type: Set(entry.actor_type.as_str().to_string()),
                auth_method: Set(entry.auth_method.clone()),
                http_method: Set(entry.http_method.clone()),
                http_path: Set(entry.http_path.clone()),
                route_pattern: Set(entry.route_pattern.clone()),
                http_status: Set(entry.http_status as i32),
                client_ip: Set(entry.client_ip.clone()),
                user_agent: Set(entry.user_agent.clone()),
                duration_ms: Set(entry.duration_ms as i64),
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
                auth_method: Set(entry.auth_method.clone()),
                http_method: Set(entry.http_method.clone()),
                http_path: Set(entry.http_path.clone()),
                route_pattern: Set(entry.route_pattern.clone()),
                http_status: Set(entry.http_status as i32),
                client_ip: Set(entry.client_ip.clone()),
                user_agent: Set(entry.user_agent.clone()),
                duration_ms: Set(entry.duration_ms as i64),
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
        tracing::info!(
            target: "uptrakit_audit",
            audit_id = %entry.id,
            tenant_id = ?entry.tenant_id,
            actor_id = %entry.actor_id,
            actor_type = %entry.actor_type,
            auth_method = %entry.auth_method,
            http_method = %entry.http_method,
            http_path = %entry.http_path,
            route_pattern = ?entry.route_pattern,
            http_status = entry.http_status,
            client_ip = ?entry.client_ip,
            user_agent = ?entry.user_agent,
            duration_ms = entry.duration_ms,
            occurred_at = %entry.occurred_at,
            "audit log entry"
        );
        Ok(())
    }
}
