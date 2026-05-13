//! DB-driven config reconciler: polls `settings_version` rows and emits
//! [`ReloadRequest`]s when versions bump.
//!
//! [`spawn_config_reconciler`] is the production entry point.  It spawns a
//! background task that ticks every [`RECONCILER_POLL`] (2 s) and compares
//! the current DB `global_version` / per-tenant `version` values against a
//! lock-free [`SettingsVersionCache`].  When a bump is detected, a
//! [`ReloadRequest`] with [`ReloadSource::DbBump`] is sent to the coordinator.
//!
//! The task runs until the database connection is dropped or the cancellation
//! token fires.  Poll errors are logged as warnings and retried on the next
//! tick.

use rootcause::prelude::*;
use sea_orm::{DatabaseConnection, EntityTrait};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use uptrakit_config_reload::config::Scope;
use uptrakit_config_reload::coordinator::{ReloadRequest, ReloadSource};
use uptrakit_config_reload::defaults::RECONCILER_POLL;
use uptrakit_config_reload::reconciler::SettingsVersionCache;
use uptrakit_shared_db::entity::settings_version;

/// Spawn the config reconciler background task.
///
/// Returns a `JoinHandle` that resolves when the task exits.  The task stops
/// cleanly when `cancel` is triggered or when the sender side of `tx` is
/// dropped (coordinator exited).
pub(crate) fn spawn_config_reconciler(
    db: DatabaseConnection,
    tx: mpsc::Sender<ReloadRequest>,
    cache: SettingsVersionCache,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RECONCILER_POLL);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    debug!("config reconciler shutting down");
                    break;
                }
                _ = tick.tick() => {}
            }

            match poll_once(&db, &tx, &cache).await {
                Ok(()) => {}
                Err(e) => {
                    warn!(error = %e, "config reconciler poll failed; retrying next tick");
                }
            }
        }
    })
}

/// Single reconciliation tick: fetch all `settings_version` rows, compare
/// against the cache, emit `ReloadRequest`s for each bump detected.
///
/// # Errors
///
/// Returns an error if the DB query fails; the caller logs and retries.
async fn poll_once(
    db: &DatabaseConnection,
    tx: &mpsc::Sender<ReloadRequest>,
    cache: &SettingsVersionCache,
) -> Result<(), Report> {
    let rows = settings_version::Entity::find()
        .all(db)
        .await
        .map_err(|e| rootcause::report!(e))?;

    for row in rows {
        let tenant_scope = Scope::Tenant(row.tenant_id);
        let new_global = u64::try_from(row.global_version).unwrap_or(0);
        let new_tenant = u64::try_from(row.version).unwrap_or(0);

        let prior_global = cache.get(Scope::Global).unwrap_or(0);
        let prior_tenant = cache.get(tenant_scope).unwrap_or(0);

        if new_global > prior_global {
            cache.update(Scope::Global, new_global);
            let req = ReloadRequest {
                source: ReloadSource::DbBump {
                    scope: Scope::Global,
                    sections: vec!["audit".into(), "registration".into()],
                },
                timestamp: OffsetDateTime::now_utc(),
            };
            // A send error means the coordinator exited; stop the loop.
            if tx.send(req).await.is_err() {
                debug!("coordinator channel closed; reconciler exiting");
                return Ok(());
            }
        }

        if new_tenant > prior_tenant {
            cache.update(tenant_scope, new_tenant);
            let req = ReloadRequest {
                source: ReloadSource::DbBump {
                    scope: tenant_scope,
                    sections: vec!["audit_log".into()],
                },
                timestamp: OffsetDateTime::now_utc(),
            };
            if tx.send(req).await.is_err() {
                debug!("coordinator channel closed; reconciler exiting");
                return Ok(());
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ActiveValue, Database};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    async fn build_test_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory DB");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migration");
        db
    }

    async fn insert_tenant_and_version(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        version: i64,
        global_version: i64,
    ) {
        use uptrakit_shared_db::entity::tenant;

        // Insert tenant first (FK constraint).
        let tenant = tenant::ActiveModel {
            id: ActiveValue::Set(tenant_id),
            name: ActiveValue::Set("test-tenant".into()),
            slug: ActiveValue::Set(tenant_id.to_string()),
            is_default: ActiveValue::Set(false),
            created_at: ActiveValue::Set(OffsetDateTime::now_utc()),
            updated_at: ActiveValue::Set(OffsetDateTime::now_utc()),
            deactivated_at: ActiveValue::Set(None),
        };
        tenant.insert(db).await.expect("insert tenant");

        // Insert settings_version row.
        let sv = settings_version::ActiveModel {
            tenant_id: ActiveValue::Set(tenant_id),
            version: ActiveValue::Set(version),
            global_version: ActiveValue::Set(global_version),
            revocation_version: ActiveValue::Set(0),
            updated_at: ActiveValue::Set(OffsetDateTime::now_utc()),
        };
        sv.insert(db).await.expect("insert settings_version");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_detects_global_bump() {
        let db = build_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant_and_version(&db, tenant_id, 1, 1).await;

        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();

        let cancel = CancellationToken::new();
        let handle = spawn_config_reconciler(db.clone(), tx, cache.clone(), cancel.clone());

        // Wait for the reconciler to detect the initial global bump.
        let req = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert!(matches!(
            req.source,
            ReloadSource::DbBump {
                scope: Scope::Global,
                ..
            }
        ));

        cancel.cancel();
        handle.await.expect("task panicked");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_detects_tenant_bump() {
        let db = build_test_db().await;
        let tenant_id = Uuid::now_v7();
        // global_version = 0 → no global bump; version = 1 → tenant bump.
        insert_tenant_and_version(&db, tenant_id, 1, 0).await;

        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();

        let cancel = CancellationToken::new();
        let handle = spawn_config_reconciler(db.clone(), tx, cache.clone(), cancel.clone());

        let req = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        assert!(matches!(
            req.source,
            ReloadSource::DbBump {
                scope: Scope::Tenant(_),
                ..
            }
        ));

        cancel.cancel();
        handle.await.expect("task panicked");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reconciler_does_not_re_emit_on_unchanged_version() {
        let db = build_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant_and_version(&db, tenant_id, 1, 1).await;

        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();

        let cancel = CancellationToken::new();
        let handle = spawn_config_reconciler(db.clone(), tx, cache.clone(), cancel.clone());

        // First tick emits bumps for both global and tenant (both are 1 > 0).
        // Drain until the channel goes quiet within one poll window.
        loop {
            let got = tokio::time::timeout(
                RECONCILER_POLL + std::time::Duration::from_millis(300),
                rx.recv(),
            )
            .await;
            if got.is_err() {
                break; // Timed out — initial bursts drained.
            }
        }

        // After the cache is primed, no further bump should be emitted for
        // the same version numbers.  Give the reconciler two more ticks.
        let nothing = tokio::time::timeout(RECONCILER_POLL * 2, rx.recv()).await;
        assert!(nothing.is_err(), "expected timeout, not a second bump");

        cancel.cancel();
        handle.await.expect("task panicked");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_once_emits_nothing_on_empty_db() {
        let db = build_test_db().await;
        let (tx, mut rx) = mpsc::channel(8);
        let cache = SettingsVersionCache::new();

        poll_once(&db, &tx, &cache).await.unwrap();
        drop(tx);

        assert!(rx.recv().await.is_none(), "expected no messages");
    }
}
