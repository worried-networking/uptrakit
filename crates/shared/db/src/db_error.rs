use sea_orm::{DbErr, RuntimeErr};

/// Returns `true` when `e` represents a unique-constraint violation.
///
/// Delegates to [`sqlx::error::ErrorKind::UniqueViolation`] so the mapping
/// from backend-specific error codes (SQLite 2067/1555, PostgreSQL 23505,
/// MySQL 1062) is handled by sqlx and not duplicated here.
pub fn is_unique_constraint_violation(e: &DbErr) -> bool {
    if let DbErr::Query(RuntimeErr::SqlxError(sqlx_err)) = e
        && let sqlx::Error::Database(dbe) = &**sqlx_err
    {
        return dbe.kind() == sqlx::error::ErrorKind::UniqueViolation;
    }
    false
}
