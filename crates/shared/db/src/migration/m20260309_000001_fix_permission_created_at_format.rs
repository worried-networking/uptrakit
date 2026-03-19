use sea_orm::{ConnectionTrait as _, DatabaseBackend, Statement, TransactionTrait as _};
use sea_orm_migration::prelude::*;

/// Fix `created_at` values in the `permissions` table that were stored in
/// `time::OffsetDateTime::Display` format instead of RFC 3339.
///
/// ## Background
///
/// Migrations m20260307_000002 and m20260308_000001 originally used
/// `execute_unprepared(&format!("INSERT … VALUES … '{now}'"))`.
/// `time::OffsetDateTime::Display` formats the UTC offset as `+HH:MM:SS`
/// (three components), producing e.g.:
///
/// ```text
/// 2026-03-02 22:33:15.239039 +00:00:00
/// ```
///
/// sqlx's `Decode<Sqlite>` for `OffsetDateTime` tries RFC 3339 first, then a
/// custom format that only accepts two-component offsets (`+HH:MM`).  The
/// three-component offset causes the decode to fail:
///
/// ```text
/// ColumnDecode { source: "invalid offset datetime: 2026-03-02 22:33:15.239039 +00:00:00" }
/// ```
///
/// This error was previously masked: the TEXT-stored UUIDs in `role_permissions`
/// (fixed by m20260308_000002) prevented those permissions from being queried.
/// Once the UUIDs were repaired, permissions became queryable and the datetime
/// decode error surfaced.
///
/// ## What this migration does
///
/// For each `permissions` row whose `created_at` contains a three-component
/// UTC offset (` +00:00:00`), re-encodes the value as RFC 3339
/// (`YYYY-MM-DDTHH:MM:SS.ffffffZ`).
///
/// ## Scope
///
/// SQLite-only.  PostgreSQL uses native timestamp types and is unaffected.
///
/// ## Down
///
/// No-op: reverting to the broken format would re-introduce the decode error.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        if db.get_database_backend() != DatabaseBackend::Sqlite {
            return Ok(());
        }

        // Find permissions with a three-component UTC offset.
        //
        // `LIKE` on a TEXT column is a SQLite-specific pattern check with no
        // sea_query equivalent for the value content; query_all_raw is the
        // approved exception.  See docs/development/database-migrations.md.
        let broken_rows = db
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT id, created_at FROM permissions \
                 WHERE created_at LIKE '% +%:%:%'",
            ))
            .await?;

        if broken_rows.is_empty() {
            return Ok(());
        }

        let mut to_fix: Vec<(Vec<u8>, String)> = Vec::with_capacity(broken_rows.len());
        for row in &broken_rows {
            use sea_orm::TryGetable as _;
            let id_bytes: Vec<u8> = Vec::<u8>::try_get_by_index(row, 0).map_err(|e| {
                DbErr::Custom(format!("failed to read permission id as bytes: {e:?}"))
            })?;
            let created_at: String = String::try_get_by_index(row, 1).map_err(|e| {
                DbErr::Custom(format!("failed to read created_at as string: {e:?}"))
            })?;
            let fixed = fix_utc_datetime(&created_at).ok_or_else(|| {
                DbErr::Custom(format!(
                    "repair migration: cannot convert created_at '{created_at}' to RFC 3339"
                ))
            })?;
            to_fix.push((id_bytes, fixed));
        }

        let txn = db.begin().await?;

        for (id_bytes, fixed_created_at) in to_fix {
            txn.execute(
                &Query::update()
                    .table(Alias::new("permissions"))
                    .value(Alias::new("created_at"), fixed_created_at)
                    .and_where(Expr::col(Alias::new("id")).eq(Value::Bytes(Some(id_bytes))))
                    .to_owned(),
            )
            .await?;
        }

        txn.commit().await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

/// Convert a `time::OffsetDateTime::Display`-formatted UTC datetime to RFC 3339.
///
/// Input:  `"2026-03-02 22:33:15.239039 +00:00:00"`
/// Output: `"2026-03-02T22:33:15.239039Z"`
///
/// Returns `None` if the input does not end with ` +00:00:00` (non-UTC or
/// unrecognized format).
fn fix_utc_datetime(s: &str) -> Option<String> {
    let bare = s
        .strip_suffix(" +00:00:00")
        .or_else(|| s.strip_suffix(" -00:00:00"))?;
    // Replace the one space between date and time with `T`.
    Some(bare.replacen(' ', "T", 1) + "Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_utc_datetime_converts_known_format() {
        assert_eq!(
            fix_utc_datetime("2026-03-02 22:33:15.239039 +00:00:00").as_deref(),
            Some("2026-03-02T22:33:15.239039Z"),
        );
    }

    #[test]
    fn fix_utc_datetime_handles_minus_zero() {
        assert_eq!(
            fix_utc_datetime("2026-03-02 22:33:15.239039 -00:00:00").as_deref(),
            Some("2026-03-02T22:33:15.239039Z"),
        );
    }

    #[test]
    fn fix_utc_datetime_returns_none_for_rfc3339() {
        assert!(fix_utc_datetime("2026-03-02T22:33:15.239039Z").is_none());
    }

    #[test]
    fn fix_utc_datetime_returns_none_for_two_component_offset() {
        assert!(fix_utc_datetime("2026-03-02 22:33:15.239039 +00:00").is_none());
    }
}
