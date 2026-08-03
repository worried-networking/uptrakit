use sea_orm::TryGetable;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Backfill an additive `mcp:use` access grant for the three roles that hold
/// the legacy `access_mcp` permission (`m20260424_000001_access_mcp_permission.rs:82`:
/// `viewer`, `operator`, `software_manager`).
///
/// M1.2's seed migration (`m20260728_000002_seed_access_grants.rs`) maps no
/// permission to `mcp:use`, so the M1.5 gate conversion would deny every MCP
/// principal until this backfill runs. This migration seeds ONE additive
/// grant row per role, keyed idempotent on the `description` marker rather
/// than plain `subject_type=role` existence — every role already has an M1.2
/// seed grant row, so a bare existence check would always skip.
///
/// Role ids are resolved by name against the live `roles` table
/// (check-then-insert idempotency; all lookups/inserts here use typed
/// builders, never interpolated raw SQL).
///
/// `down()` deletes by a TIGHT predicate (subject_type role + the three role
/// ids + the description marker) so it can never sweep other grant rows.
/// Dev/test reversal only; production seed rollback is unsupported.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// Roles that hold legacy `access_mcp`
/// (`m20260424_000001_access_mcp_permission.rs:82`).
const BACKFILL_ROLES: &[&str] = &["viewer", "operator", "software_manager"];

/// Idempotence key: the M1.2 seed rows for these roles carry `description =
/// NULL`, so this marker distinguishes the backfill row from the seed row.
const BACKFILL_DESCRIPTION: &str = "mcp:use seed backfill (M1.5)";

fn all_selector() -> serde_json::Value {
    serde_json::json!({ "type": "all" })
}

async fn role_id_by_name(manager: &SchemaManager<'_>, name: &str) -> Result<Option<Uuid>, DbErr> {
    let row = manager
        .get_connection()
        .query_one(
            &Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("roles"))
                .and_where(Expr::col(Alias::new("name")).eq(name))
                .to_owned(),
        )
        .await?;
    match row {
        None => Ok(None),
        Some(row) => Uuid::try_get_by_index(&row, 0)
            .map(Some)
            .map_err(|e| DbErr::Custom(format!("seed role `{name}` id unreadable: {e:?}"))),
    }
}

async fn backfill_grant_exists(manager: &SchemaManager<'_>, role_id: Uuid) -> Result<bool, DbErr> {
    let row = manager
        .get_connection()
        .query_one(
            &Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("access_grants"))
                .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                .and_where(Expr::col(Alias::new("subject_id")).eq(role_id))
                .and_where(Expr::col(Alias::new("description")).eq(BACKFILL_DESCRIPTION))
                .limit(1)
                .to_owned(),
        )
        .await?;
    Ok(row.is_some())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();
        let mut insert = Query::insert()
            .into_table(Alias::new("access_grants"))
            .columns([
                Alias::new("id"),
                Alias::new("tenant_id"),
                Alias::new("subject_type"),
                Alias::new("subject_id"),
                Alias::new("patterns"),
                Alias::new("selector"),
                Alias::new("description"),
                Alias::new("created_at"),
                Alias::new("updated_at"),
                Alias::new("created_by"),
            ])
            .to_owned();
        let mut pending = 0usize;

        for role_name in BACKFILL_ROLES {
            let role_id = role_id_by_name(manager, role_name).await?.ok_or_else(|| {
                DbErr::Custom(format!("backfill role `{role_name}` is missing from roles"))
            })?;
            if backfill_grant_exists(manager, role_id).await? {
                continue;
            }
            insert.values_panic([
                Expr::value(Uuid::now_v7()),
                Expr::value(Option::<Uuid>::None),
                Expr::value("role"),
                Expr::value(role_id),
                Expr::value(serde_json::Value::from(vec!["mcp:use".to_string()])),
                Expr::value(all_selector()),
                Expr::value(Some(BACKFILL_DESCRIPTION.to_string())),
                Expr::value(now),
                Expr::value(now),
                Expr::value(Option::<Uuid>::None),
            ]);
            pending += 1;
        }

        // One accumulated insert (batch invariant); an INSERT with zero VALUES
        // rows is a syntax error on both backends, hence the guard.
        if pending > 0 {
            manager.exec_stmt(insert).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut doomed: Vec<Uuid> = Vec::new();
        for role_name in BACKFILL_ROLES {
            let Some(role_id) = role_id_by_name(manager, role_name).await? else {
                continue;
            };
            let rows = manager
                .get_connection()
                .query_all(
                    &Query::select()
                        .column(Alias::new("id"))
                        .from(Alias::new("access_grants"))
                        .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                        .and_where(Expr::col(Alias::new("subject_id")).eq(role_id))
                        .and_where(Expr::col(Alias::new("description")).eq(BACKFILL_DESCRIPTION))
                        .to_owned(),
                )
                .await?;
            for row in rows {
                let id = Uuid::try_get_by_index(&row, 0)
                    .map_err(|e| DbErr::Custom(format!("grant id unreadable: {e:?}")))?;
                doomed.push(id);
            }
        }
        if !doomed.is_empty() {
            manager
                .exec_stmt(
                    Query::delete()
                        .from_table(Alias::new("access_grants"))
                        .and_where(Expr::col(Alias::new("id")).is_in(doomed))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::str::FromStr;

    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, TryGetable};
    use sea_orm_migration::prelude::*;
    use uptrakit_shared_types::access::ActionPattern;
    use uuid::Uuid;

    use super::{BACKFILL_DESCRIPTION, BACKFILL_ROLES, Migration};
    use crate::migration::Migrator;

    const ALL_BUILT_IN_ROLES: &[&str] = &[
        "viewer",
        "operator",
        "service_manager",
        "software_manager",
        "host_manager",
        "settings_manager",
        "command_manager",
        "system_administrator",
    ];

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    async fn role_ids_by_name(db: &DatabaseConnection) -> BTreeMap<String, Uuid> {
        let rows = db
            .query_all(
                &Query::select()
                    .columns([Alias::new("id"), Alias::new("name")])
                    .from(Alias::new("roles"))
                    .to_owned(),
            )
            .await
            .expect("select roles");
        rows.iter()
            .map(|row| {
                let id = Uuid::try_get_by_index(row, 0).expect("role id");
                let name = String::try_get_by_index(row, 1).expect("role name");
                (name, id)
            })
            .collect()
    }

    /// subject_id → patterns, scoped to backfill-marked rows only.
    async fn backfill_rows(db: &DatabaseConnection) -> BTreeMap<Uuid, BTreeSet<String>> {
        let rows = db
            .query_all(
                &Query::select()
                    .columns([Alias::new("subject_id"), Alias::new("patterns")])
                    .from(Alias::new("access_grants"))
                    .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                    .and_where(Expr::col(Alias::new("description")).eq(BACKFILL_DESCRIPTION))
                    .to_owned(),
            )
            .await
            .expect("select backfill access_grants");
        rows.iter()
            .map(|row| {
                let subject_id = Uuid::try_get_by_index(row, 0).expect("subject_id");
                let patterns: serde_json::Value =
                    serde_json::Value::try_get_by_index(row, 1).expect("patterns json");
                let set: BTreeSet<String> = patterns
                    .as_array()
                    .expect("patterns must be a JSON array")
                    .iter()
                    .map(|v| v.as_str().expect("pattern must be a string").to_string())
                    .collect();
                (subject_id, set)
            })
            .collect()
    }

    /// After migrations, each of the three roles has a backfill grant with
    /// `patterns == ["mcp:use"]`; the other five built-in roles have none.
    #[tokio::test]
    async fn backfill_grants_mcp_use_to_access_mcp_roles_only() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("migrations apply");
        let roles = role_ids_by_name(&db).await;
        let grants = backfill_rows(&db).await;
        assert_eq!(grants.len(), 3, "exactly one backfill grant per role");

        for role_name in BACKFILL_ROLES {
            let role_id = roles.get(*role_name).expect("backfill role exists");
            let set = grants.get(role_id).expect("role has a backfill grant");
            let expected: BTreeSet<String> = ["mcp:use".to_string()].into_iter().collect();
            assert_eq!(set, &expected, "pattern set for {role_name}");
        }

        for role_name in ALL_BUILT_IN_ROLES {
            if BACKFILL_ROLES.contains(role_name) {
                continue;
            }
            let role_id = roles.get(*role_name).expect("role exists");
            assert!(
                !grants.contains_key(role_id),
                "{role_name} must not receive a backfill grant"
            );
        }
    }

    /// Re-running `up()` inserts nothing (idempotence keyed on the
    /// description marker).
    #[tokio::test]
    async fn backfill_up_is_idempotent() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("migrations apply");
        let before = backfill_rows(&db).await;

        let schema_manager = SchemaManager::new(&db);
        Migration.up(&schema_manager).await.expect("re-apply up");

        let after = backfill_rows(&db).await;
        assert_eq!(before, after, "re-running up() must insert nothing");
    }

    /// Parse guard: `"mcp:use"` parses and matches the live catalog (mirrors
    /// the sibling's `seed_patterns_stay_valid_against_live_catalog` test).
    #[test]
    fn mcp_use_pattern_parses_and_matches_live_catalog() {
        let parsed = ActionPattern::from_str("mcp:use");
        assert!(parsed.is_ok(), "`mcp:use` must parse: {parsed:?}");
        let pattern = parsed.expect("checked above");
        assert!(
            pattern.can_match_any(),
            "`mcp:use` must match at least one live-catalog action"
        );
    }

    /// Seed-down deletes exactly the backfill rows: a differently-marked
    /// role grant (e.g. an M1.2 seed row, description NULL) must survive.
    #[tokio::test]
    async fn backfill_down_uses_tight_predicate() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("migrations apply");
        let roles = role_ids_by_name(&db).await;
        let viewer_id = *roles.get("viewer").expect("viewer role");

        // Sanity: the M1.2 seed row for viewer (description NULL) exists
        // before down() runs.
        let seed_count_before = db
            .query_all(
                &Query::select()
                    .expr(Func::count(Expr::col(Alias::new("id"))))
                    .from(Alias::new("access_grants"))
                    .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                    .and_where(Expr::col(Alias::new("subject_id")).eq(viewer_id))
                    .and_where(Expr::col(Alias::new("description")).is_null())
                    .to_owned(),
            )
            .await
            .expect("count seed rows");
        let seed_count_before: i64 =
            i64::try_get_by_index(seed_count_before.first().expect("count row present"), 0)
                .expect("count value");
        assert_eq!(seed_count_before, 1, "viewer must have its M1.2 seed row");

        let schema_manager = SchemaManager::new(&db);
        Migration
            .down(&schema_manager)
            .await
            .expect("backfill down");

        let remaining = backfill_rows(&db).await;
        assert!(
            remaining.is_empty(),
            "down() must remove all backfill-marked rows"
        );

        let seed_count_after = db
            .query_all(
                &Query::select()
                    .expr(Func::count(Expr::col(Alias::new("id"))))
                    .from(Alias::new("access_grants"))
                    .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                    .and_where(Expr::col(Alias::new("subject_id")).eq(viewer_id))
                    .and_where(Expr::col(Alias::new("description")).is_null())
                    .to_owned(),
            )
            .await
            .expect("count seed rows");
        let seed_count_after: i64 =
            i64::try_get_by_index(seed_count_after.first().expect("count row present"), 0)
                .expect("count value");
        assert_eq!(
            seed_count_after, 1,
            "the M1.2 seed row (description NULL) must survive down()"
        );
    }
}
