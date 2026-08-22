//! Query helpers for the global `users` table.
//!
//! [`find_by_canonical_email`] is the sole sanctioned comparison site for
//! `user::Column::Email` (enforced by `ci/verify_email_canonical_ingress.sh`).
//! `MaskedEmail` values are canonical by construction (`FromStr` trims and
//! ASCII-lowercases), and the stored column is canonical after the
//! canonicalize-user-emails migration, so plain equality on the canonical
//! value is a case-insensitive match.

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use thiserror::Error;
use uptrakit_shared_types::MaskedEmail;

use crate::entity::user;

/// Errors from the `users` query helpers.
#[derive(Debug, Error)]
pub enum UsersError {
    /// A database error occurred. No `#[from]`: all conversions route
    /// through `.context_to()` via `impl_report_conversion!` below, and
    /// error-handling.md bans carrying both (the `From` impl would be dead
    /// code).
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<UsersError>>;

uptrakit_shared_macros::impl_report_conversion!(sea_orm::DbErr => UsersError::Db);

/// Find a user by canonical email. The `users` table is global (not
/// tenant-scoped), so no tenant filter applies.
///
/// # Errors
///
/// Returns [`UsersError::Db`] for any database error from the underlying
/// query.
pub async fn find_by_canonical_email<C: ConnectionTrait>(
    db: &C,
    email: &MaskedEmail,
) -> Result<Option<user::Model>> {
    user::Entity::find()
        .filter(user::Column::Email.eq(email))
        .one(db)
        .await
        .context_to()
}
