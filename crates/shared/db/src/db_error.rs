use sea_orm::DbErr;
#[cfg(any(feature = "db-sqlite", feature = "db-postgres"))]
use sea_orm::RuntimeErr;

/// Returns `true` when `e` represents a unique-constraint violation.
///
/// Delegates to [`sqlx::error::ErrorKind::UniqueViolation`] so the mapping
/// from backend-specific error codes (SQLite 2067/1555, PostgreSQL 23505,
/// MySQL 1062) is handled by sqlx and not duplicated here.
///
/// When no sqlx-backed database feature (`db-sqlite` or `db-postgres`) is
/// enabled, this always returns `false` because the sqlx error type is not
/// available.
pub fn is_unique_constraint_violation(e: &DbErr) -> bool {
    #[cfg(any(feature = "db-sqlite", feature = "db-postgres"))]
    if let DbErr::Query(RuntimeErr::SqlxError(sqlx_err)) = e
        && let sqlx::Error::Database(dbe) = &**sqlx_err
    {
        return dbe.kind() == sqlx::error::ErrorKind::UniqueViolation;
    }
    let _ = e;
    false
}
