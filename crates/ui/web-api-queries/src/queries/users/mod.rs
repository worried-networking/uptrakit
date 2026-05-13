//! User-related database query helpers.

#[cfg(feature = "oidc")]
pub mod oidc_sync;

use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::user;
use uuid::Uuid;

/// A snapshot of a user entity for audit purposes.
///
/// Omits `password_hash` (secret) and timestamp fields (auto-skipped by macro).
#[derive(uptrakit_audit_log::AuditView)]
#[audit(target_type = "user")]
pub struct UserView {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub is_active: bool,
}

impl From<&user::Model> for UserView {
    fn from(m: &user::Model) -> Self {
        Self {
            id: m.id,
            email: m.email.expose_email().to_string(),
            first_name: m.first_name.clone(),
            last_name: m.last_name.clone(),
            is_active: m.is_active,
        }
    }
}

/// Fetch the user by `user_id` inside an existing transaction, update
/// `is_active` (and `deactivated_at` / `updated_at`), and return the
/// before/after model pair.  Returns `Ok(None)` when the user does not exist.
pub async fn update_user_active_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    user_id: Uuid,
    is_active: bool,
) -> Result<Option<(user::Model, user::Model)>, sea_orm::DbErr> {
    let Some(before) = user::Entity::find_by_id(user_id).one(tx).await? else {
        return Ok(None);
    };
    let now = OffsetDateTime::now_utc();
    let mut active: user::ActiveModel = before.clone().into();
    active.is_active = Set(is_active);
    active.deactivated_at = Set(if is_active { None } else { Some(now) });
    active.updated_at = Set(now);
    let after = active.update(tx).await?;
    Ok(Some((before, after)))
}
