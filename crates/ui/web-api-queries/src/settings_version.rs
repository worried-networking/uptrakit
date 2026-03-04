//! Standalone version-counter bumping for the `settings_version` table.
//!
//! Extracted here so query modules (`services.rs`) can bump the revocation
//! counter without depending on the full `settings_store` module in web-api.

use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, ExprTrait, QueryFilter, Set,
    sea_query::{Expr, OnConflict},
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{prelude::*, settings_version};
use uuid::Uuid;

/// Atomically bump the revocation version counter after a certificate revocation.
///
/// Non-fatal on failure: callers should log and continue.
pub async fn bump_revocation_version(
    db: &impl ConnectionTrait,
    tenant_id: Uuid,
) -> Result<(), sea_orm::DbErr> {
    let now = OffsetDateTime::now_utc();

    let result = SettingsVersion::update_many()
        .col_expr(
            settings_version::Column::RevocationVersion,
            Expr::col(settings_version::Column::RevocationVersion).add(1),
        )
        .col_expr(settings_version::Column::UpdatedAt, Expr::value(now))
        .filter(settings_version::Column::TenantId.eq(tenant_id))
        .exec(db)
        .await?;

    // Defensive: if the row didn't exist (tenant created after migration), insert it.
    // Use on_conflict(do_nothing) to avoid racing with a concurrent insert.
    if result.rows_affected == 0 {
        let model = settings_version::ActiveModel {
            tenant_id: Set(tenant_id),
            version: Set(0),
            global_version: Set(0),
            revocation_version: Set(1),
            updated_at: Set(now),
        };
        SettingsVersion::insert(model)
            .on_conflict(
                OnConflict::column(settings_version::Column::TenantId)
                    .do_nothing()
                    .to_owned(),
            )
            .try_insert()
            .exec(db)
            .await?;
    }

    Ok(())
}
