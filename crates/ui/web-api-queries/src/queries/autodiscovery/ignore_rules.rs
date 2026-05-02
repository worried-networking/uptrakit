//! Ignore rule CRUD operations for autodiscovery.

use super::{AutodiscoveryError, Result};
use rootcause::prelude::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use uptrakit_shared_db::entity::{prelude::*, software_ignore};
use uptrakit_shared_db::is_unique_constraint_violation;
use uptrakit_web_api_types::autodiscovery::SoftwareIgnoreResponse;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uuid::Uuid;

/// Insert an autodiscovery ignore rule by software item name (idempotent).
///
/// Returns `true` if a new rule was inserted, `false` if the rule already
/// existed (including the case where a concurrent request inserted the same
/// rule between our call and the DB write).
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn create_or_ignore_ignore_rule(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    name: &str,
    host_id: Option<Uuid>,
) -> Result<bool> {
    let record = software_ignore::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id),
        host_id: Set(host_id),
        name: Set(name.to_string()),
        created_at: Set(time::OffsetDateTime::now_utc()),
    };

    match SoftwareIgnore::insert(record).exec(db).await {
        Ok(_) => Ok(true),
        Err(e) if is_unique_constraint_violation(&e) => Ok(false),
        Err(e) => Err(report!(AutodiscoveryError::Db(e))),
    }
}

/// List autodiscovery ignore rules for a tenant.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn list_ignore_rules(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    params: &PaginationParams,
) -> Result<PaginatedResponse<SoftwareIgnoreResponse>> {
    use sea_orm::PaginatorTrait;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 1000);

    let paginator = SoftwareIgnore::find()
        .filter(software_ignore::Column::TenantId.eq(tenant_id))
        .order_by_desc(software_ignore::Column::CreatedAt)
        .paginate(db, per_page);

    let total = paginator.num_items().await.context_to()?;
    let items_raw = paginator.fetch_page(page - 1).await.context_to()?;

    let items = items_raw
        .into_iter()
        .map(|r| SoftwareIgnoreResponse {
            id: r.id,
            name: r.name,
            host_id: r.host_id,
            created_at: r.created_at,
        })
        .collect::<Vec<_>>();

    let total_pages = total.div_ceil(per_page);

    Ok(PaginatedResponse {
        items,
        total,
        page,
        per_page,
        total_pages,
    })
}

/// Hard-delete an autodiscovery ignore rule.
///
/// Returns `true` if a row was deleted, `false` if the rule was not found
/// for this tenant.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn delete_ignore_rule(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<bool> {
    let result = SoftwareIgnore::delete_many()
        .filter(software_ignore::Column::Id.eq(id))
        .filter(software_ignore::Column::TenantId.eq(tenant_id))
        .exec(db)
        .await
        .context_to()?;
    Ok(result.rows_affected > 0)
}

/// Hard-delete multiple autodiscovery ignore rules.
///
/// Returns `(succeeded_ids, failed)` where `failed` contains `(id, reason)` pairs.
#[expect(clippy::type_complexity, reason = "complex SeaORM query return type")]
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn batch_delete_ignore_rules(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    // Load existing rules to determine which IDs are valid.
    let rules = SoftwareIgnore::find()
        .filter(software_ignore::Column::Id.is_in(ids.iter().copied()))
        .filter(software_ignore::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .context_to()?;

    let found: std::collections::HashSet<Uuid> = rules.into_iter().map(|r| r.id).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    // Delete all found rules in one query.
    if !found.is_empty() {
        SoftwareIgnore::delete_many()
            .filter(software_ignore::Column::Id.is_in(found.iter().copied()))
            .filter(software_ignore::Column::TenantId.eq(tenant_id))
            .exec(db)
            .await
            .context_to()?;
        succeeded.extend(found);
    }

    Ok((succeeded, failed))
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::queries::autodiscovery::tests_common::{insert_tenant, setup_db};
    use sea_orm::PaginatorTrait;

    #[tokio::test]
    async fn insert_new_rule_returns_true() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let inserted = create_or_ignore_ignore_rule(&db, tenant_id, "FreshRSS", None)
            .await
            .expect("should succeed");

        assert!(inserted, "first insert must return true");

        let count = SoftwareIgnore::find()
            .filter(software_ignore::Column::TenantId.eq(tenant_id))
            .filter(software_ignore::Column::Name.eq("FreshRSS"))
            .count(&db)
            .await
            .expect("count");
        assert_eq!(count, 1, "must have created exactly one row");
    }

    #[tokio::test]
    async fn insert_duplicate_rule_returns_false() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let first = create_or_ignore_ignore_rule(&db, tenant_id, "FreshRSS", None)
            .await
            .expect("first call");
        assert!(first, "first call must return true (new row)");

        let second = create_or_ignore_ignore_rule(&db, tenant_id, "FreshRSS", None)
            .await
            .expect("second call");
        assert!(
            !second,
            "second call must return false (duplicate suppressed)"
        );

        // Exactly one row must exist after both calls.
        let count = SoftwareIgnore::find()
            .filter(software_ignore::Column::TenantId.eq(tenant_id))
            .filter(software_ignore::Column::Name.eq("FreshRSS"))
            .count(&db)
            .await
            .expect("count");
        assert_eq!(count, 1, "duplicate insert must not create a second row");
    }
}
