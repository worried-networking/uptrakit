//! Controller backstop: reap `InProgress` updates that exceeded their budget.
//!
//! This is the liveness backstop. An update may stay `InProgress` forever if the
//! agent dies mid-execution without ever sending a terminal `UpdateResult`, and
//! the owner-aware reconnect path never fires (e.g. the agent never reconnects).
//! [`reap_overdue_updates`] keys purely on the update's own budget — never on
//! agent connectivity — so a stuck update is eventually forced to a terminal
//! `Interrupted` state.

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, sea_query::Expr};
use time::OffsetDateTime;
use uptrakit_shared_db::begin_immediate;
use uptrakit_shared_db::entity::update_history;
use uptrakit_shared_db::entity::{service, service_host};
use uuid::Uuid;

use crate::queries::update_dispatch::TriggerUpdateError;

pub(crate) const RECOVERY_HINT: &str = "execution outcome unknown — connection lost or deadline exceeded; \
     verify the installed version before re-running";
const REASON: &str = "Update interrupted: deadline exceeded without a terminal result";

pub(crate) const PENDING_RECOVERY_HINT: &str = "the update never started on the host — the dispatch was not delivered before the \
     agent connection was lost; safe to re-run once the agent reconnects";
const PENDING_REASON: &str =
    "Update interrupted: dispatch never started — agent connection absent past the dispatch grace";

/// A live (non-deactivated) service linked to a stalled row's host, carrying
/// the persisted liveness signal the reaper's evidence predicate consumes.
#[derive(Clone)]
pub struct StalledCandidateService {
    pub service_id: Uuid,
    /// `services.last_seen_at` — refreshed on every service ping
    /// (`record_service_activity`); `None` = never seen since enrollment.
    pub last_seen_at: Option<OffsetDateTime>,
}

/// A `Pending` row past the dispatch grace, with its host's live linked
/// services — the caller applies connection-absent evidence before committing
/// the reap.
pub struct StalledPendingCandidate {
    pub row: update_history::Model,
    /// Live services linked to the row's host via `service_host` (may be
    /// empty — an unlinked host has no delivery path at all).
    pub services: Vec<StalledCandidateService>,
}

/// Mark every `in_progress` update whose deadline (`started_at` + its own
/// budget + `grace`) is past `now` as terminal `Interrupted`.
///
/// Each row's own budget (`timeout_seconds`) is honored; `default_budget` is
/// the fallback applied to legacy rows with a `NULL` `timeout_seconds`.
///
/// Returns the reaped rows so the caller can emit notifications/SSE. Keys only on
/// the update's own budget — never on agent connectivity. `now` is injected so
/// tests drive the wall-clock independently of the tokio clock.
///
/// `Queued`/`Pending`/`AwaitingRestart` rows are left untouched: only
/// `InProgress` rows with a non-null `started_at` past their deadline are
/// reaped.
pub async fn reap_overdue_updates(
    db: &DatabaseConnection,
    now: OffsetDateTime,
    default_budget: std::time::Duration,
    grace: std::time::Duration,
) -> std::result::Result<Vec<update_history::Model>, rootcause::Report<TriggerUpdateError>> {
    // Ceiling for per-row budgets on read. Bounds two hazards from a corrupt
    // or absurd `timeout_seconds` value (e.g. a row carrying the write-side
    // `i64::MAX` fallback): the `OffsetDateTime` deadline addition below
    // overflowing, and the row wedging its host's one-active-update slot as
    // unreapable forever.
    const MAX_BUDGET_SECS: u64 = 60 * 60 * 24 * 30; // 30 days

    // Read-then-write: open an IMMEDIATE transaction BEFORE the read so the
    // select and the update share one write-intent txn (coding-standards.md
    // SQLite rule — BEGIN DEFERRED would risk SQLITE_BUSY_SNAPSHOT). Mirrors
    // `maybe_complete_batch` in `update_batches/dispatch.rs`.
    let txn = begin_immediate(db).await.context_to()?;

    // ponytail: loads every InProgress row and filters in memory — bounded by
    // the one-active-update-per-host invariant (row count <= host count).
    // Move the cutoff back into SQL only if that invariant ever changes.
    let rows = update_history::Entity::find()
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::InProgress))
        .filter(update_history::Column::StartedAt.is_not_null())
        .all(&txn)
        .await
        .context_to()?;

    if rows.is_empty() {
        txn.commit().await.context_to()?;
        return Ok(vec![]);
    }

    let overdue: Vec<update_history::Model> = rows
        .into_iter()
        .filter(|row| {
            // NULL = legacy row created before per-row budgets; fall back to
            // the caller's default. New rows always write a value.
            let budget = row
                .timeout_seconds
                .and_then(|s| u64::try_from(s).ok())
                .map_or(default_budget, |s| {
                    std::time::Duration::from_secs(s.min(MAX_BUDGET_SECS))
                });
            // checked_add, never a bare `started + duration` (panics on
            // overflow; workspace builds abort on panic). An unrepresentable
            // deadline means the row is corrupt — treat it as overdue so it
            // cannot wedge the host's one-active-update slot.
            row.started_at.is_some_and(|started| {
                time::Duration::try_from(budget + grace)
                    .ok()
                    .and_then(|d| started.checked_add(d))
                    .is_none_or(|deadline| deadline < now)
            })
        })
        .collect();

    if overdue.is_empty() {
        txn.commit().await.context_to()?;
        return Ok(vec![]);
    }

    let ids: Vec<Uuid> = overdue.iter().map(|r| r.id).collect();
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
    let reaped = overdue
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

/// List `Pending` rows whose `created_at` is older than `grace` (wall-clock
/// `now` injected for tests), with each row's live host→service links and
/// their `last_seen_at` batch-loaded.
///
/// `Queued` rows are left to batch promotion (ADR-0024); `InProgress` rows are
/// the existing budget reaper's business. Read-only — the caller decides which
/// candidates have connection-absent evidence and passes the survivors to
/// [`reap_stalled_pending_updates`].
pub async fn list_stalled_pending_updates(
    db: &DatabaseConnection,
    now: OffsetDateTime,
    grace: std::time::Duration,
) -> std::result::Result<Vec<StalledPendingCandidate>, rootcause::Report<TriggerUpdateError>> {
    let cutoff = now - grace;
    let rows = update_history::Entity::find()
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .filter(update_history::Column::CreatedAt.lt(cutoff))
        .all(db)
        .await
        .context_to()?;
    if rows.is_empty() {
        return Ok(vec![]);
    }

    let host_ids: Vec<Uuid> = rows.iter().map(|r| r.host_id).collect();
    let links = service_host::Entity::find()
        .filter(service_host::Column::HostId.is_in(host_ids))
        .all(db)
        .await
        .context_to()?;

    // Resolve link liveness in one batched query, keeping only live
    // (non-deactivated) services. A deactivated service is not a delivery
    // path and must not block the reap.
    let service_ids: Vec<Uuid> = links.iter().map(|l| l.service_id).collect();
    let live: std::collections::HashMap<Uuid, Option<OffsetDateTime>> = service::Entity::find()
        .filter(service::Column::Id.is_in(service_ids))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|s| (s.id, s.last_seen_at))
        .collect();

    let mut by_host: std::collections::HashMap<Uuid, Vec<StalledCandidateService>> =
        std::collections::HashMap::new();
    for link in links {
        if let Some(last_seen_at) = live.get(&link.service_id) {
            by_host
                .entry(link.host_id)
                .or_default()
                .push(StalledCandidateService {
                    service_id: link.service_id,
                    last_seen_at: *last_seen_at,
                });
        }
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let services = by_host.get(&row.host_id).cloned().unwrap_or_default();
            StalledPendingCandidate { row, services }
        })
        .collect())
}

/// CAS-flip the given rows to terminal `Interrupted`, only where still
/// `Pending` — a row that started executing between the caller's list and this
/// write is skipped by the status filter (same CAS pattern as
/// [`reap_overdue_updates`]). Returns the models actually reaped, patched to
/// the written state so the caller can emit events.
pub async fn reap_stalled_pending_updates(
    db: &DatabaseConnection,
    now: OffsetDateTime,
    ids: &[Uuid],
) -> std::result::Result<Vec<update_history::Model>, rootcause::Report<TriggerUpdateError>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let txn = begin_immediate(db).await.context_to()?;

    // Re-read under the write txn so the returned models reflect exactly the
    // rows the CAS below will hit.
    let candidates = update_history::Entity::find()
        .filter(update_history::Column::Id.is_in(ids.to_vec()))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .all(&txn)
        .await
        .context_to()?;
    if candidates.is_empty() {
        txn.commit().await.context_to()?;
        return Ok(vec![]);
    }

    let reap_ids: Vec<Uuid> = candidates.iter().map(|r| r.id).collect();
    update_history::Entity::update_many()
        .filter(update_history::Column::Id.is_in(reap_ids))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::Interrupted),
        )
        .col_expr(update_history::Column::CompletedAt, Expr::value(Some(now)))
        .col_expr(
            update_history::Column::RecoveryHint,
            Expr::value(Some(PENDING_RECOVERY_HINT.to_string())),
        )
        .col_expr(
            update_history::Column::Output,
            Expr::value(PENDING_REASON.to_string()),
        )
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(PENDING_REASON.len() as i64),
        )
        .exec(&txn)
        .await
        .context_to()?;
    txn.commit().await.context_to()?;

    Ok(candidates
        .into_iter()
        .map(|mut r| {
            r.status = update_history::UpdateStatus::Interrupted;
            r.completed_at = Some(now);
            r.recovery_hint = Some(PENDING_RECOVERY_HINT.to_string());
            r.output = PENDING_REASON.to_string();
            r.output_bytes = PENDING_REASON.len() as i64;
            r
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queries::update_batches::tests::{Fixture, insert_base_fixture, setup_db};
    use sea_orm::{ActiveModelTrait, Set};
    use time::Duration as TDuration;
    use uptrakit_shared_db::entity::host;
    use uptrakit_shared_db::entity::service::ServiceStatus;

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
        timeout_seconds: Option<i64>,
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
            timeout_seconds: Set(timeout_seconds),
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
            None,
        )
        .await;
        // (b) in_progress started 1min ago (fresh) -> left alone.
        let fresh = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::InProgress,
            Some(now - TDuration::minutes(1)),
            now - TDuration::minutes(1),
            None,
        )
        .await;
        // (c) queued -> left alone.
        let queued = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::Queued,
            None,
            now - TDuration::hours(3),
            None,
        )
        .await;
        // (d) awaiting_restart 3h ago -> left alone.
        let awaiting = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::AwaitingRestart,
            Some(now - TDuration::hours(3)),
            now - TDuration::hours(3),
            None,
        )
        .await;

        let reaped = reap_overdue_updates(
            &db,
            now,
            std::time::Duration::from_secs(7200),
            std::time::Duration::from_secs(300),
        )
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

    #[tokio::test]
    async fn respects_per_row_budget() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        // Row A: budget 60s, started 10 minutes ago -> reaped (well past its
        // own 60s + 300s grace deadline).
        let short_budget = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::InProgress,
            Some(now - TDuration::minutes(10)),
            now - TDuration::minutes(10),
            Some(60),
        )
        .await;
        // Row B: budget 86400s (1 day), started 3 hours ago -> NOT reaped
        // (inside its own budget, even though it is past the default 7200s).
        let long_budget = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::InProgress,
            Some(now - TDuration::hours(3)),
            now - TDuration::hours(3),
            Some(86400),
        )
        .await;

        let reaped = reap_overdue_updates(
            &db,
            now,
            std::time::Duration::from_secs(7200),
            std::time::Duration::from_secs(300),
        )
        .await
        .unwrap();

        assert_eq!(reaped.len(), 1, "only the short-budget row is reaped");
        assert_eq!(reaped[0].id, short_budget);
        assert_eq!(reaped[0].status, update_history::UpdateStatus::Interrupted);

        let row = update_history::Entity::find_by_id(long_budget)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.status,
            update_history::UpdateStatus::InProgress,
            "row within its own (larger) budget must be untouched"
        );
    }

    #[tokio::test]
    async fn null_budget_falls_back_to_default() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        // NULL budget, started 1 hour ago -> NOT reaped under default 7200s +
        // 300s grace (1 hour is well inside 7200s + 300s).
        let within_default = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::InProgress,
            Some(now - TDuration::hours(1)),
            now - TDuration::hours(1),
            None,
        )
        .await;
        // NULL budget, started 3 hours ago -> reaped (past the default
        // 7200s + 300s grace).
        let past_default = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::InProgress,
            Some(now - TDuration::hours(3)),
            now - TDuration::hours(3),
            None,
        )
        .await;

        let reaped = reap_overdue_updates(
            &db,
            now,
            std::time::Duration::from_secs(7200),
            std::time::Duration::from_secs(300),
        )
        .await
        .unwrap();

        assert_eq!(
            reaped.len(),
            1,
            "only the row past the default budget is reaped"
        );
        assert_eq!(reaped[0].id, past_default);

        let row = update_history::Entity::find_by_id(within_default)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.status,
            update_history::UpdateStatus::InProgress,
            "row within the default budget must be untouched"
        );
    }

    #[tokio::test]
    async fn lists_only_overdue_pending() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();
        let grace = std::time::Duration::from_secs(600);

        let overdue = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::Pending,
            None,
            now - (grace + std::time::Duration::from_secs(1)),
            None,
        )
        .await;
        let _fresh = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::Pending,
            None,
            now - std::time::Duration::from_secs(1),
            None,
        )
        .await;
        let _queued = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::Queued,
            None,
            now - (grace + std::time::Duration::from_secs(1)),
            None,
        )
        .await;

        let candidates = list_stalled_pending_updates(&db, now, grace).await.unwrap();
        let ids: Vec<Uuid> = candidates.iter().map(|c| c.row.id).collect();
        assert_eq!(ids, vec![overdue]);
    }

    #[tokio::test]
    async fn pending_reap_cas_skips_rows_that_started() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();
        let grace = std::time::Duration::from_secs(600);
        let old = now - (grace + std::time::Duration::from_secs(1));

        let stalled = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::Pending,
            None,
            old,
            None,
        )
        .await;
        let racing = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::Pending,
            None,
            old,
            None,
        )
        .await;

        // The dispatch race: `racing` starts executing between list and reap.
        let mut m: update_history::ActiveModel = update_history::Entity::find_by_id(racing)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
        m.status = Set(update_history::UpdateStatus::InProgress);
        m.started_at = Set(Some(now));
        m.update(&db).await.unwrap();

        let reaped = reap_stalled_pending_updates(&db, now, &[stalled, racing])
            .await
            .unwrap();
        assert_eq!(reaped.len(), 1);
        assert_eq!(reaped[0].id, stalled);
        assert_eq!(reaped[0].status, update_history::UpdateStatus::Interrupted);
        assert_eq!(reaped[0].output, PENDING_REASON);
        assert_eq!(
            reaped[0].recovery_hint.as_deref(),
            Some(PENDING_RECOVERY_HINT)
        );

        let untouched = update_history::Entity::find_by_id(racing)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(untouched.status, update_history::UpdateStatus::InProgress);

        // The reap is persisted, not just returned.
        let persisted = update_history::Entity::find_by_id(stalled)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(persisted.status, update_history::UpdateStatus::Interrupted);
    }

    #[tokio::test]
    async fn lists_only_live_non_deactivated_services() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();
        let grace = std::time::Duration::from_secs(600);
        let old = now - (grace + std::time::Duration::from_secs(1));

        let overdue = insert_update(
            &db,
            &f,
            update_history::UpdateStatus::Pending,
            None,
            old,
            None,
        )
        .await;
        let row = update_history::Entity::find_by_id(overdue)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let host_id = row.host_id;

        let live_service_id = Uuid::now_v7();
        let live_last_seen = now - TDuration::minutes(5);
        service::ActiveModel {
            id: Set(live_service_id),
            tenant_id: Set(f.tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("live-host".to_string()),
            friendly_name: Set("Live Service".to_string()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{live_service_id}")),
            client_version: Set(None),
            last_seen_at: Set(Some(live_last_seen)),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        service_host::ActiveModel {
            service_id: Set(live_service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(&db)
        .await
        .unwrap();

        let deactivated_service_id = Uuid::now_v7();
        service::ActiveModel {
            id: Set(deactivated_service_id),
            tenant_id: Set(f.tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("deactivated-host".to_string()),
            friendly_name: Set("Deactivated Service".to_string()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{deactivated_service_id}")),
            client_version: Set(None),
            last_seen_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(Some(now)),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        service_host::ActiveModel {
            service_id: Set(deactivated_service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(&db)
        .await
        .unwrap();

        let candidates = list_stalled_pending_updates(&db, now, grace).await.unwrap();
        let candidate = candidates
            .into_iter()
            .find(|c| c.row.id == overdue)
            .expect("overdue row must be listed");

        let service_ids: Vec<Uuid> = candidate.services.iter().map(|s| s.service_id).collect();
        assert_eq!(service_ids, vec![live_service_id]);
        assert!(!service_ids.contains(&deactivated_service_id));

        let live = candidate
            .services
            .iter()
            .find(|s| s.service_id == live_service_id)
            .unwrap();
        assert_eq!(live.last_seen_at, Some(live_last_seen));
    }
}
