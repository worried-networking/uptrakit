use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(
            Table::drop()
                .table(Alias::new("system_audit_logs"))
                .if_exists()
                .to_owned(),
        )
        .await?;
        m.drop_table(
            Table::drop()
                .table(Alias::new("audit_logs"))
                .if_exists()
                .to_owned(),
        )
        .await?;
        m.create_table(build_audit_logs()).await?;
        m.create_table(build_system_audit_logs()).await?;
        create_indexes(m).await?;
        Ok(())
    }

    // `down()` drops the V2 tables without recreating the V1 schema.
    //
    // The V2 design accepts bounded audit-history loss on rollback;
    // see docs/security/audit-logs.md for the cutover section.
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(
            Table::drop()
                .table(Alias::new("system_audit_logs"))
                .if_exists()
                .to_owned(),
        )
        .await?;
        m.drop_table(
            Table::drop()
                .table(Alias::new("audit_logs"))
                .if_exists()
                .to_owned(),
        )
        .await
    }
}

/// CHECK constraint that enforces the action_kind/snapshot invariant:
///
/// - `event`: point-in-time notification; `before_snapshot` and
///   `after_snapshot` must both be NULL.
/// - `stateful`: resource mutation; both snapshot columns must be NOT NULL.
fn audit_kind_check() -> SimpleExpr {
    // event path: kind = 'event' AND both snapshots are NULL
    let is_event = Expr::col(Alias::new("action_kind"))
        .eq("event")
        .and(Expr::col(Alias::new("before_snapshot")).is_null())
        .and(Expr::col(Alias::new("after_snapshot")).is_null());

    // stateful path: kind = 'stateful' AND both snapshots are NOT NULL
    let is_stateful = Expr::col(Alias::new("action_kind"))
        .eq("stateful")
        .and(Expr::col(Alias::new("before_snapshot")).is_not_null())
        .and(Expr::col(Alias::new("after_snapshot")).is_not_null());

    is_event.or(is_stateful)
}

fn build_audit_logs() -> TableCreateStatement {
    Table::create()
        .table(Alias::new("audit_logs"))
        .col(
            ColumnDef::new(Alias::new("id"))
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Alias::new("tenant_id")).uuid().not_null())
        .col(
            ColumnDef::new(Alias::new("occurred_at"))
                .timestamp_with_time_zone()
                .not_null(),
        )
        .col(
            ColumnDef::new(Alias::new("actor_type"))
                .string_len(32)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("actor_id")).uuid())
        .col(ColumnDef::new(Alias::new("actor_display")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("action_type"))
                .string_len(128)
                .not_null(),
        )
        .col(
            ColumnDef::new(Alias::new("action_kind"))
                .string_len(16)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("target_type")).string_len(128))
        .col(ColumnDef::new(Alias::new("target_id")).string_len(255))
        .col(ColumnDef::new(Alias::new("target_display")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("outcome"))
                .string_len(32)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("details_json")).json_binary())
        .col(ColumnDef::new(Alias::new("before_snapshot")).json_binary())
        .col(ColumnDef::new(Alias::new("after_snapshot")).json_binary())
        .col(ColumnDef::new(Alias::new("correlation_id")).uuid())
        .col(ColumnDef::new(Alias::new("request_id")).string_len(255))
        .check(audit_kind_check())
        .to_owned()
}

fn build_system_audit_logs() -> TableCreateStatement {
    Table::create()
        .table(Alias::new("system_audit_logs"))
        .col(
            ColumnDef::new(Alias::new("id"))
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(
            ColumnDef::new(Alias::new("occurred_at"))
                .timestamp_with_time_zone()
                .not_null(),
        )
        .col(
            ColumnDef::new(Alias::new("actor_type"))
                .string_len(32)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("actor_id")).uuid())
        .col(ColumnDef::new(Alias::new("actor_display")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("action_type"))
                .string_len(128)
                .not_null(),
        )
        .col(
            ColumnDef::new(Alias::new("action_kind"))
                .string_len(16)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("target_type")).string_len(128))
        .col(ColumnDef::new(Alias::new("target_id")).string_len(255))
        .col(ColumnDef::new(Alias::new("target_display")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("outcome"))
                .string_len(32)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("details_json")).json_binary())
        .col(ColumnDef::new(Alias::new("before_snapshot")).json_binary())
        .col(ColumnDef::new(Alias::new("after_snapshot")).json_binary())
        .col(ColumnDef::new(Alias::new("correlation_id")).uuid())
        .col(ColumnDef::new(Alias::new("request_id")).string_len(255))
        .check(audit_kind_check())
        .to_owned()
}

async fn create_indexes(m: &SchemaManager<'_>) -> Result<(), DbErr> {
    for (name, cols) in [
        ("idx_audit_tenant_time", &["tenant_id", "occurred_at"][..]),
        (
            "idx_audit_tenant_action_time",
            &["tenant_id", "action_type", "occurred_at"],
        ),
        (
            "idx_audit_tenant_actor_time",
            &["tenant_id", "actor_type", "occurred_at"],
        ),
        (
            "idx_audit_tenant_outcome_time",
            &["tenant_id", "outcome", "occurred_at"],
        ),
        (
            "idx_audit_tenant_target_type_time",
            &["tenant_id", "target_type", "occurred_at"],
        ),
        (
            "idx_audit_tenant_target_id_time",
            &["tenant_id", "target_id", "occurred_at"],
        ),
        (
            "idx_audit_tenant_actor_id_time",
            &["tenant_id", "actor_id", "occurred_at"],
        ),
        (
            "idx_audit_tenant_correlation",
            &["tenant_id", "correlation_id"],
        ),
        (
            "idx_audit_tenant_kind_time",
            &["tenant_id", "action_kind", "occurred_at"],
        ),
    ] {
        create_index(m, "audit_logs", cols, name).await?;
    }

    for (name, cols) in [
        ("idx_system_audit_time", &["occurred_at"][..]),
        (
            "idx_system_audit_action_time",
            &["action_type", "occurred_at"],
        ),
        (
            "idx_system_audit_actor_time",
            &["actor_type", "occurred_at"],
        ),
        ("idx_system_audit_outcome_time", &["outcome", "occurred_at"]),
        (
            "idx_system_audit_target_type_time",
            &["target_type", "occurred_at"],
        ),
        (
            "idx_system_audit_target_id_time",
            &["target_id", "occurred_at"],
        ),
        (
            "idx_system_audit_actor_id_time",
            &["actor_id", "occurred_at"],
        ),
        ("idx_system_audit_correlation", &["correlation_id"]),
        (
            "idx_system_audit_kind_time",
            &["action_kind", "occurred_at"],
        ),
    ] {
        create_index(m, "system_audit_logs", cols, name).await?;
    }

    Ok(())
}

async fn create_index(
    m: &SchemaManager<'_>,
    table: &str,
    cols: &[&str],
    name: &str,
) -> Result<(), DbErr> {
    let mut idx = Index::create();
    idx.name(name).table(Alias::new(table));
    for &col in cols {
        if col == "occurred_at" {
            idx.col((Alias::new(col), IndexOrder::Desc));
        } else {
            idx.col(Alias::new(col));
        }
    }
    m.create_index(idx.to_owned()).await
}
