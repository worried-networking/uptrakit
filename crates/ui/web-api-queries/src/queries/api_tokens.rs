//! Database query helpers for the `api_token` entity.
//!
//! All mutation helpers accept a `&sea_orm::DatabaseTransaction` opened as
//! `BEGIN IMMEDIATE` by the caller so that the audit row can be written in the
//! same transaction (`emit_stateful`).

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::api_token;
use uuid::Uuid;

// ── Audit snapshot ────────────────────────────────────────────────────────────

/// Audit snapshot for an API token.
///
/// `token_hash` is explicitly excluded: it is a SHA-256 digest of the secret
/// bearer token and must never appear in audit log snapshots.
#[derive(uptrakit_audit_log::AuditView)]
#[audit(target_type = "api_token")]
pub struct ApiTokenView {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    #[audit(skip)]
    pub token_hash: String,
    pub created_at: time::OffsetDateTime,
    pub last_used_at: Option<time::OffsetDateTime>,
    pub revoked_at: Option<time::OffsetDateTime>,
}

impl From<&api_token::Model> for ApiTokenView {
    fn from(m: &api_token::Model) -> Self {
        Self {
            id: m.id,
            user_id: m.user_id,
            name: m.name.clone(),
            token_hash: m.token_hash.clone(),
            created_at: m.created_at,
            last_used_at: m.last_used_at,
            revoked_at: m.revoked_at,
        }
    }
}

// ── Transaction-aware variants (for emit_stateful callers) ───────────────────

/// Insert a new API token inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// The caller is responsible for generating `id`, `token_hash`, and
/// `created_at` before opening the transaction so that the values are
/// available for both the DB row and the HTTP response.
///
/// # Errors
///
/// Returns `sea_orm::DbErr` if the insert fails (e.g. unique constraint on
/// `token_hash` or `name`).
pub async fn create_api_token_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    id: Uuid,
    user_id: Uuid,
    name: &str,
    token_hash: String,
    created_at: OffsetDateTime,
) -> Result<api_token::Model, sea_orm::DbErr> {
    let model = api_token::ActiveModel {
        id: Set(id),
        user_id: Set(user_id),
        name: Set(name.to_string()),
        token_hash: Set(token_hash),
        created_at: Set(created_at),
        last_used_at: Set(None),
        revoked_at: Set(None),
    };
    api_token::Entity::insert(model)
        .exec_with_returning(tx)
        .await
}

/// Revoke an API token inside a caller-managed `BEGIN IMMEDIATE` transaction.
///
/// Reads the current row, sets `revoked_at = now()`, and returns
/// `Some((before, after))`.  Returns `None` when the token does not exist,
/// belongs to a different user, or is already revoked.
///
/// # Errors
///
/// Returns `sea_orm::DbErr` if the underlying read or update fails.
pub async fn revoke_api_token_in_tx(
    tx: &sea_orm::DatabaseTransaction,
    token_id: Uuid,
    user_id: Uuid,
) -> Result<Option<(api_token::Model, api_token::Model)>, sea_orm::DbErr> {
    let Some(before) = api_token::Entity::find_by_id(token_id)
        .filter(api_token::Column::UserId.eq(user_id))
        .filter(api_token::Column::RevokedAt.is_null())
        .one(tx)
        .await?
    else {
        return Ok(None);
    };

    let mut active: api_token::ActiveModel = before.clone().into();
    active.revoked_at = Set(Some(OffsetDateTime::now_utc()));
    let after = active.update(tx).await?;

    Ok(Some((before, after)))
}
