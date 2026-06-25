//! Controller backstop: reap `InProgress` updates that exceeded their budget.
//!
//! This is the liveness backstop. An update may stay `InProgress` forever if the
//! agent dies mid-execution without ever sending a terminal `UpdateResult`, and
//! the owner-aware reconnect path never fires (e.g. the agent never reconnects).
//! [`reap_overdue_updates`] keys purely on the update's own budget — never on
//! agent connectivity — so a stuck update is eventually forced to a terminal
//! `Interrupted` state.

use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, SqliteTransactionMode,
    TransactionOptions, TransactionTrait, sea_query::Expr,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::update_history;
use uuid::Uuid;

use crate::queries::update_dispatch::TriggerUpdateError;

pub(crate) const RECOVERY_HINT: &str = "execution outcome unknown — connection lost or deadline exceeded; \
     verify the installed version before re-running";
const REASON: &str = "Update interrupted: deadline exceeded without a terminal result";

/// Mark every `in_progress` update whose `started_at` is older than `max_age`
/// (relative to `now`, wall-clock) as terminal `Interrupted`.
///
/// Returns the reaped rows so the caller can emit notifications/SSE. Keys only on
/// the update's own budget — never on agent connectivity. `now` is injected so
/// tests drive the wall-clock independently of the tokio clock.
///
/// `Queued`/`Pending`/`AwaitingRestart` rows are left untouched: only
/// `InProgress` rows with a non-null `started_at` older than the cutoff are
/// reaped.
pub async fn reap_overdue_updates(
    db: &DatabaseConnection,
    now: OffsetDateTime,
    max_age: std::time::Duration,
) -> std::result::Result<Vec<update_history::Model>, rootcause::Report<TriggerUpdateError>> {
    // `OffsetDateTime` implements `Sub<std::time::Duration>` directly — no
    // `time::Duration` conversion needed.
    let cutoff = now - max_age;

    // Read-then-write: open an IMMEDIATE transaction BEFORE the read so the
    // select and the update share one write-intent txn (coding-standards.md
    // SQLite rule — BEGIN DEFERRED would risk SQLITE_BUSY_SNAPSHOT). Mirrors
    // `maybe_complete_batch` in `update_batches/dispatch.rs`.
    let txn = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .context_to()?;

    let candidates = update_history::Entity::find()
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .filter(update_history::Column::StartedAt.is_not_null())
        .filter(update_history::Column::StartedAt.lt(cutoff))
        .all(&txn)
        .await
        .context_to()?;

    if candidates.is_empty() {
        txn.commit().await.context_to()?;
        return Ok(vec![]);
    }

    let ids: Vec<Uuid> = candidates.iter().map(|r| r.id).collect();
    update_history::Entity::update_many()
        .filter(update_history::Column::Id.is_in(ids))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::Interrupted),
        )
        .col_expr(update_history::Column::CompletedAt, Expr::value(Some(now)))
        .col_expr(
            update_history::Column::RecoveryHint,
            Expr::value(Some(RECOVERY_HINT.to_string())),
        )
        .col_expr(
            update_history::Column::Output,
            Expr::value(REASON.to_string()),
        )
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(REASON.len() as i64),
        )
        .exec(&txn)
        .await
        .context_to()?;
    txn.commit().await.context_to()?;

    // Patch the in-memory models to match what was written, so the caller's
    // event emission sees the terminal state.
    let reaped = candidates
        .into_iter()
        .map(|mut r| {
            r.status = update_history::UpdateStatus::Interrupted;
            r.completed_at = Some(now);
            r.recovery_hint = Some(RECOVERY_HINT.to_string());
            r.output = REASON.to_string();
            r.output_bytes = REASON.len() as i64;
            r
        })
        .collect();
    Ok(reaped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queries::update_batches::tests::{Fixture, insert_base_fixture, setup_db};
    use sea_orm::{ActiveModelTrait, Set};
    use time::Duration as TDuration;
    use uptrakit_shared_db::entity::host;

    /// Insert a distinct host for the fixture's tenant and return its id.
    /// A partial unique index (`uix_update_history_host_active`) forbids more than
    /// one active (`pending`/`in_progress`/`awaiting_restart`) `update_history`
    /// row per `host_id`, so each active test row needs its own host.
    async fn insert_host(db: &DatabaseConnection, f: &Fixture) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        host::ActiveModel {
            id: Set(id),
            tenant_id: Set(f.tenant_id),
            machine_id: Set(format!("machine-{id}")),
            hostname: Set(format!("host-{id}")),
            friendly_name: Set(format!("Host {id}")),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        id
    }

    /// Insert one `update_history` row with the given status and `started_at`,
    /// on a distinct host to satisfy the active-row unique index.
    async fn insert_update(
        db: &DatabaseConnection,
        f: &Fixture,
        status: update_history::UpdateStatus,
        started_at: Option<OffsetDateTime>,
        created_at: OffsetDateTime,
    ) -> Uuid {
        let id = Uuid::now_v7();
        let host_id = insert_host(db, f).await;
        update_history::ActiveModel {
            id: Set(id),
            tenant_id: Set(f.tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(f.item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(status),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(started_at),
            completed_at: Set(None),
            awaiting_restart_since: Set(None),
            created_at: Set(created_at),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn reaps_only_overdue_in_progress() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        // (a) in_progress started 3h ago (overdue) -> reaped.
        let overdue = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::InProgress,
            Some(now - TDuration::hours(3)),
            now - TDuration::hours(3),
        )
        .await;
        // (b) in_progress started 1min ago (fresh) -> left alone.
        let fresh = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::InProgress,
            Some(now - TDuration::minutes(1)),
            now - TDuration::minutes(1),
        )
        .await;
        // (c) queued -> left alone.
        let queued = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::Queued,
            None,
            now - TDuration::hours(3),
        )
        .await;
        // (d) awaiting_restart 3h ago -> left alone.
        let awaiting = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::AwaitingRestart,
            Some(now - TDuration::hours(3)),
            now - TDuration::hours(3),
        )
        .await;

        let reaped = reap_overdue_updates(&db, now, std::time::Duration::from_secs(7200 + 300))
            .await
            .unwrap();

        assert_eq!(
            reaped.len(),
            1,
            "only the overdue in_progress row is reaped"
        );
        assert_eq!(reaped[0].id, overdue);

        let row = update_history::Entity::find_by_id(overdue)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, update_history::UpdateStatus::Interrupted);
        assert!(row.completed_at.is_some());
        assert!(row.recovery_hint.is_some());
        assert_eq!(row.output, REASON);
        assert_eq!(row.output_bytes, REASON.len() as i64);

        // The other three rows are untouched.
        for (id, expected) in [
            (fresh, update_history::UpdateStatus::InProgress),
            (queued, update_history::UpdateStatus::Queued),
            (awaiting, update_history::UpdateStatus::AwaitingRestart),
        ] {
            let row = update_history::Entity::find_by_id(id)
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, expected, "row {id} must be untouched");
            assert!(row.completed_at.is_none());
            assert!(row.recovery_hint.is_none());
        }
    }
}
