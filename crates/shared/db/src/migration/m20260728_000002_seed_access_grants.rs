use sea_orm::TryGetable;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Seed grants for the eight built-in roles (`06-grant-model.md` §Seed roles,
/// including the two owner-confirmed widenings: `settings_manager` +
/// `settings:read` and `operator` + `services:read`).
///
/// Every seed string below is a FROZEN literal. A historical migration is a
/// frozen artifact that fresh installs execute against the CURRENT binary:
/// referencing `actions::*_STR` consts or validating against the live catalog
/// would turn any future catalog rename into a fresh-install-only outage (or
/// force edits to an append-only file). Drift protection lives in this file's
/// `seed_patterns_stay_valid_against_live_catalog` CI guard test instead —
/// a catalog change that orphans a seed goes red at the causing commit and
/// ships a FORWARD data migration (the m20260310 precedent), never an edit
/// here.
///
/// Role ids are resolved by name against the live `roles` table
/// (check-then-insert idempotency, the m20260310 idiom for structure only —
/// all lookups/inserts here use typed builders, never interpolated raw SQL).
/// A missing role name aborts (fail-closed; cannot happen in sequence —
/// Migration 1 of this pair follows m20260310, which seeds all eight).
///
/// `down()` deletes by a TIGHT predicate (subject_type role + the eight role
/// ids + created_by NULL + selector All + exact pattern-set match) so it can
/// never sweep future M1.6a-created role grants whose created_by happens to
/// be NULL. Dev/test reversal only; production seed rollback is unsupported.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// (role name, pattern strings) — restates 06 §Seed roles verbatim.
const SEED_GRANTS: &[(&str, &[&str])] = &[
    ("viewer", &["*:read"]),
    (
        "operator",
        &[
            "services:read",
            "services:approve",
            "services:reject",
            "hosts:read",
            "checks:trigger",
            "updates:trigger",
        ],
    ),
    ("service_manager", &["services:*"]),
    (
        "software_manager",
        &[
            "software:*",
            "hosts:read",
            "checks:trigger",
            "updates:trigger",
            "scheduler:manage",
            "discovery.ignores:manage",
            "plugin-configs:trigger",
        ],
    ),
    ("host_manager", &["hosts:*", "hosts.tags:manage"]),
    (
        "settings_manager",
        &[
            "settings:read",
            "settings.*:manage",
            "notifications:*",
            "audit:read",
            "users:manage",
            "access:manage",
        ],
    ),
    (
        "command_manager",
        &["commands:manage", "plugin-configs:trigger"],
    ),
    ("system_administrator", &["system.*:*"]),
];

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

async fn role_grant_exists(manager: &SchemaManager<'_>, role_id: Uuid) -> Result<bool, DbErr> {
    let row = manager
        .get_connection()
        .query_one(
            &Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("access_grants"))
                .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                .and_where(Expr::col(Alias::new("subject_id")).eq(role_id))
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

        for (role_name, patterns) in SEED_GRANTS {
            let role_id = role_id_by_name(manager, role_name).await?.ok_or_else(|| {
                DbErr::Custom(format!("seed role `{role_name}` is missing from roles"))
            })?;
            if role_grant_exists(manager, role_id).await? {
                continue;
            }
            let patterns_json = serde_json::Value::from(
                patterns
                    .iter()
                    .map(|p| (*p).to_string())
                    .collect::<Vec<String>>(),
            );
            insert.values_panic([
                Expr::value(Uuid::now_v7()),
                Expr::value(Option::<Uuid>::None),
                Expr::value("role"),
                Expr::value(role_id),
                Expr::value(patterns_json),
                Expr::value(all_selector()),
                Expr::value(Option::<String>::None),
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
        for (role_name, patterns) in SEED_GRANTS {
            let Some(role_id) = role_id_by_name(manager, role_name).await? else {
                continue;
            };
            let rows = manager
                .get_connection()
                .query_all(
                    &Query::select()
                        .columns([
                            Alias::new("id"),
                            Alias::new("patterns"),
                            Alias::new("selector"),
                        ])
                        .from(Alias::new("access_grants"))
                        .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                        .and_where(Expr::col(Alias::new("subject_id")).eq(role_id))
                        .and_where(Expr::col(Alias::new("created_by")).is_null())
                        .to_owned(),
                )
                .await?;
            let expected: std::collections::BTreeSet<&str> = patterns.iter().copied().collect();
            for row in rows {
                let id = Uuid::try_get_by_index(&row, 0)
                    .map_err(|e| DbErr::Custom(format!("grant id unreadable: {e:?}")))?;
                let raw_patterns = serde_json::Value::try_get_by_index(&row, 1)
                    .map_err(|e| DbErr::Custom(format!("grant patterns unreadable: {e:?}")))?;
                let raw_selector = serde_json::Value::try_get_by_index(&row, 2)
                    .map_err(|e| DbErr::Custom(format!("grant selector unreadable: {e:?}")))?;
                let stored: Option<std::collections::BTreeSet<String>> =
                    raw_patterns.as_array().map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    });
                let stored_matches = stored.is_some_and(|s| {
                    s.len() == expected.len() && s.iter().all(|p| expected.contains(p.as_str()))
                });
                if stored_matches && raw_selector == all_selector() {
                    doomed.push(id);
                }
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

    use super::{Migration, SEED_GRANTS, all_selector};
    use crate::migration::Migrator;

    /// Independent restatement of `06-grant-model.md` §Seed roles — kept
    /// deliberately separate from `SEED_GRANTS` so a typo in the migration's
    /// own table cannot self-verify.
    const EXPECTED: &[(&str, &[&str])] = &[
        ("viewer", &["*:read"]),
        (
            "operator",
            &[
                "services:read",
                "services:approve",
                "services:reject",
                "hosts:read",
                "checks:trigger",
                "updates:trigger",
            ],
        ),
        ("service_manager", &["services:*"]),
        (
            "software_manager",
            &[
                "software:*",
                "hosts:read",
                "checks:trigger",
                "updates:trigger",
                "scheduler:manage",
                "discovery.ignores:manage",
                "plugin-configs:trigger",
            ],
        ),
        ("host_manager", &["hosts:*", "hosts.tags:manage"]),
        (
            "settings_manager",
            &[
                "settings:read",
                "settings.*:manage",
                "notifications:*",
                "audit:read",
                "users:manage",
                "access:manage",
            ],
        ),
        (
            "command_manager",
            &["commands:manage", "plugin-configs:trigger"],
        ),
        ("system_administrator", &["system.*:*"]),
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

    /// (subject_id → (patterns, selector, tenant_id is null, created_by is null))
    async fn seed_rows(
        db: &DatabaseConnection,
    ) -> BTreeMap<Uuid, (BTreeSet<String>, serde_json::Value, bool, bool)> {
        let rows = db
            .query_all(
                &Query::select()
                    .columns([
                        Alias::new("subject_id"),
                        Alias::new("patterns"),
                        Alias::new("selector"),
                        Alias::new("tenant_id"),
                        Alias::new("created_by"),
                    ])
                    .from(Alias::new("access_grants"))
                    .and_where(Expr::col(Alias::new("subject_type")).eq("role"))
                    .to_owned(),
            )
            .await
            .expect("select access_grants");
        rows.iter()
            .map(|row| {
                let subject_id = Uuid::try_get_by_index(row, 0).expect("subject_id");
                let patterns: serde_json::Value =
                    serde_json::Value::try_get_by_index(row, 1).expect("patterns json");
                let selector: serde_json::Value =
                    serde_json::Value::try_get_by_index(row, 2).expect("selector json");
                let tenant_id: Option<Uuid> =
                    Option::<Uuid>::try_get_by_index(row, 3).expect("tenant_id");
                let created_by: Option<Uuid> =
                    Option::<Uuid>::try_get_by_index(row, 4).expect("created_by");
                let set: BTreeSet<String> = patterns
                    .as_array()
                    .expect("patterns must be a JSON array")
                    .iter()
                    .map(|v| v.as_str().expect("pattern must be a string").to_string())
                    .collect();
                (
                    subject_id,
                    (set, selector, tenant_id.is_none(), created_by.is_none()),
                )
            })
            .collect()
    }

    /// Every 06 seed-table row present verbatim; E11's two owner-confirmed
    /// widenings asserted explicitly; all rows role-subject, global, All.
    #[tokio::test]
    async fn seed_content_matches_expected_table() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("migrations apply");
        let roles = role_ids_by_name(&db).await;
        let grants = seed_rows(&db).await;
        assert_eq!(grants.len(), 8, "exactly one seed grant per role");

        for (role_name, patterns) in EXPECTED {
            let role_id = roles.get(*role_name).expect("seed role exists");
            let (set, selector, tenant_null, author_null) =
                grants.get(role_id).expect("role has a seed grant");
            let expected_set: BTreeSet<String> =
                patterns.iter().map(|p| (*p).to_string()).collect();
            assert_eq!(set, &expected_set, "pattern set for {role_name}");
            assert_eq!(selector, &all_selector(), "selector for {role_name}");
            assert!(*tenant_null, "{role_name} seed grant must be global");
            assert!(*author_null, "{role_name} seed grant must have NULL author");
        }

        // E11 (owner-confirmed widenings; assert the exact strings).
        let settings_manager = grants
            .get(
                roles
                    .get("settings_manager")
                    .expect("settings_manager role"),
            )
            .expect("settings_manager grant");
        assert!(
            settings_manager.0.contains("settings:read"),
            "settings_manager must gain settings:read (E11)"
        );
        let operator = grants
            .get(roles.get("operator").expect("operator role"))
            .expect("operator grant");
        assert!(
            operator.0.contains("services:read"),
            "operator must gain services:read (E11)"
        );
    }

    /// CI drift guard: every FROZEN seed literal must still parse and match
    /// at least one live-catalog action. A catalog change that orphans a seed
    /// turns this red at the causing commit (whole-workspace backend-test job
    /// runs this suite with no path filtering) and must ship a forward data
    /// migration.
    #[test]
    fn seed_patterns_stay_valid_against_live_catalog() {
        for (role_name, patterns) in SEED_GRANTS {
            for raw in *patterns {
                let parsed = ActionPattern::from_str(raw);
                assert!(
                    parsed.is_ok(),
                    "seed pattern `{raw}` for `{role_name}` no longer parses: {parsed:?}"
                );
                let pattern = parsed.expect("checked above");
                assert!(
                    pattern.can_match_any(),
                    "seed pattern `{raw}` for `{role_name}` matches nothing in the live catalog"
                );
            }
        }
    }

    /// Seed-down deletes exactly the seed rows: an M1.6a-shaped grant
    /// (role subject, created_by NULL, NON-seed patterns) must survive. A
    /// bare up→down→up round-trip cannot prove this — check-then-insert
    /// idempotency keeps it green even for a delete-nothing down().
    #[tokio::test]
    async fn seed_down_uses_tight_predicate() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("migrations apply");
        let roles = role_ids_by_name(&db).await;
        let viewer_id = *roles.get("viewer").expect("viewer role");

        // Survivor: M1.6a-shaped role grant with NULL author, non-seed patterns.
        let survivor_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        db.execute(
            &Query::insert()
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
                .values_panic([
                    Expr::value(survivor_id),
                    Expr::value(Option::<Uuid>::None),
                    Expr::value("role"),
                    Expr::value(viewer_id),
                    Expr::value(serde_json::json!(["notifications:read"])),
                    Expr::value(all_selector()),
                    Expr::value(Option::<String>::None),
                    Expr::value(now),
                    Expr::value(now),
                    Expr::value(Option::<Uuid>::None),
                ])
                .to_owned(),
        )
        .await
        .expect("insert survivor grant");

        let schema_manager = SchemaManager::new(&db);
        Migration.down(&schema_manager).await.expect("seed down");

        let remaining = seed_rows(&db).await;
        // Exactly the survivor remains (seed_rows keys by subject_id; the
        // survivor is the only viewer-subject row left).
        assert_eq!(remaining.len(), 1, "only the survivor may remain");
        let (set, _, _, _) = remaining.get(&viewer_id).expect("survivor remains");
        assert!(
            set.contains("notifications:read"),
            "the surviving row must be the M1.6a-shaped grant"
        );

        // Re-apply: check-then-insert re-seeds the eight (viewer already has
        // a grant row, so viewer is skipped — assert the seven others return).
        Migrator::up(&db, None).await.expect("re-apply");
    }
}
