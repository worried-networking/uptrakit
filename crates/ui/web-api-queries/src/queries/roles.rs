//! Role CRUD queries (M1.6a).
//!
//! The tenant-mixed `roles` table stays outside `TenantDb` helpers (like
//! `access_grants`): global rows are `tenant_id NULL` (the built-ins),
//! custom roles carry the owning tenant. Every query scopes explicitly.
//! All functions are `ConnectionTrait`-generic so role deletion runs
//! inside the lockout-guard transaction together with
//! `uptrakit_shared_db::access_grants::delete_grants_for_role`.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{role, user_role};
use uuid::Uuid;

#[cfg(all(test, feature = "db-sqlite"))]
mod tests;

/// Errors returned by role queries.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RoleQueryError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
    /// The role does not exist in the caller's scope.
    #[error("role not found")]
    NotFound,
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, rootcause::Report<RoleQueryError>>;

uptrakit_shared_macros::impl_report_conversion!(sea_orm::DbErr => RoleQueryError::Db);

/// Which scope an existing role name collides in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleNameCollision {
    /// Collides with a global (built-in) role name — rejected outright
    /// (shadowing a global name would poison by-name resolvers).
    Global,
    /// Collides with another custom role in the same tenant.
    Tenant,
}

fn scope_condition(tenant_id: Uuid) -> Condition {
    Condition::any()
        .add(role::Column::TenantId.is_null())
        .add(role::Column::TenantId.eq(tenant_id))
}

/// Global built-ins plus the tenant's custom roles, name-ordered.
pub async fn list_roles<C: ConnectionTrait>(db: &C, tenant_id: Uuid) -> Result<Vec<role::Model>> {
    role::Entity::find()
        .filter(scope_condition(tenant_id))
        .order_by_asc(role::Column::Name)
        .all(db)
        .await
        .context_to()
}

/// A single role visible in the tenant's scope.
pub async fn get_role<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    role_id: Uuid,
) -> Result<role::Model> {
    role::Entity::find_by_id(role_id)
        .filter(scope_condition(tenant_id))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(RoleQueryError::NotFound))
}

/// Name-collision probe for create/rename. `exclude_role_id` skips the
/// role being renamed (rename-to-own-name is legal).
///
/// # Accepted risk (recorded)
///
/// This probe is a separate statement from the [`create_role`] insert and
/// the [`update_role`] update, so it is not atomic (TOCTOU): two concurrent
/// creates/renames of the same name can both see "no collision", and the
/// loser's write then trips `uix_roles_global_name`/`uix_roles_tenant_name`
/// and arrives as an untyped [`RoleQueryError::Db`] — there is deliberately
/// no typed collision variant, because the unique index carries no scope
/// information a caller could map back to [`RoleNameCollision`]. Callers
/// must not treat a clean probe as a guarantee that the following write
/// succeeds. Role writes are an infrequent admin path; the index, not this
/// probe, is the correctness boundary. Plan 2 owns the HTTP mapping (409 on
/// a probe hit, 500 on the racing-write path) — do not build locking here.
pub async fn find_role_name_collision<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    name: &str,
    exclude_role_id: Option<Uuid>,
) -> Result<Option<RoleNameCollision>> {
    let mut query = role::Entity::find()
        .filter(role::Column::Name.eq(name))
        .filter(scope_condition(tenant_id));
    if let Some(exclude) = exclude_role_id {
        query = query.filter(role::Column::Id.ne(exclude));
    }
    let hits = query.all(db).await.context_to()?;
    if hits.iter().any(|r| r.tenant_id.is_none()) {
        return Ok(Some(RoleNameCollision::Global));
    }
    if !hits.is_empty() {
        return Ok(Some(RoleNameCollision::Tenant));
    }
    Ok(None)
}

/// Create a tenant-scoped custom role.
pub async fn create_role<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    name: &str,
    description: Option<String>,
) -> Result<role::Model> {
    role::ActiveModel {
        id: Set(Uuid::now_v7()),
        name: Set(name.to_string()),
        description: Set(description),
        is_built_in: Set(false),
        created_at: Set(OffsetDateTime::now_utc()),
        tenant_id: Set(Some(tenant_id)),
    }
    .insert(db)
    .await
    .context_to()
}

/// Resolve a role OWNED by `tenant_id` (custom roles only — built-ins are
/// global rows and never match). Every mutation path routes through this
/// so a handler that forgets its own checks cannot rename or delete a
/// built-in or another tenant's role at the query layer.
async fn get_own_role<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    role_id: Uuid,
) -> Result<role::Model> {
    role::Entity::find_by_id(role_id)
        .filter(role::Column::TenantId.eq(tenant_id))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(RoleQueryError::NotFound))
}

/// Rename/re-describe an OWN-tenant custom role. `tenant_id` and
/// `is_built_in` are immutable (spec §Role CRUD); built-ins and
/// foreign-tenant roles are `NotFound` here (handlers 409 built-ins before
/// calling — this is the query-layer backstop). Returns the (before,
/// after) snapshot pair for the Stateful audit emit.
pub async fn update_role<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    role_id: Uuid,
    name: &str,
    description: Option<String>,
) -> Result<(role::Model, role::Model)> {
    let before = get_own_role(db, tenant_id, role_id).await?;
    let mut active: role::ActiveModel = before.clone().into();
    active.name = Set(name.to_string());
    active.description = Set(description);
    let after = active.update(db).await.context_to()?;
    Ok((before, after))
}

/// Delete an OWN-tenant custom role row and its `user_roles` assignments
/// (same backstop scoping as [`update_role`]). Grant cleanup
/// (`access_grants::delete_grants_for_role`) is the caller's obligation in
/// the same transaction, and must run AFTER this call: that function takes
/// no tenant argument and checks no ownership, so the OWN-tenant resolution
/// performed here is what makes it safe.
pub async fn delete_role_rows<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    role_id: Uuid,
) -> Result<()> {
    let role = get_own_role(db, tenant_id, role_id).await?;
    // `user_role` is `TenantScoped`, so its writes carry the tenant filter by rule.
    // The filter is not load-bearing here and no test can discriminate it: the
    // (unnamed) `user_roles.role_id -> roles.id` foreign key is `ON DELETE CASCADE`
    // (`m20260209_000001_initial.rs`, the `UserRoles` table create), so the `roles`
    // delete below removes every remaining assignment anyway — including any row
    // whose `tenant_id` disagrees with its role's (no composite FK ties the two).
    // The explicit delete stays so the intent is visible at the query layer and
    // does not silently depend on cascade configuration.
    user_role::Entity::delete_many()
        .filter(user_role::Column::RoleId.eq(role.id))
        .filter(user_role::Column::TenantId.eq(tenant_id))
        .exec(db)
        .await
        .context_to()?;
    role::Entity::delete_by_id(role.id)
        .exec(db)
        .await
        .context_to()?;
    Ok(())
}
