use crate::AppState;
use crate::middleware::tenant_context::TenantContext;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::IntoResponse;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DeleteMany, PrimaryKeyTrait, QueryFilter, Select, UpdateMany,
};
use std::sync::Arc;
use uptrakit_shared_db::entity::TenantScoped;
use uuid::Uuid;

/// Combined database connection + tenant identity extractor.
///
/// Route handlers that deal with tenant-scoped data should use this extractor
/// instead of accessing `state.db` directly. Tenant-agnostic routes (e.g. PKI,
/// system health) should continue to use `State(state)` and call `state.db()`.
pub struct TenantDb {
    db: DatabaseConnection,
    pub tenant_id: Uuid,
}

impl TenantDb {
    /// Returns a reference to the underlying database connection.
    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Return a `SELECT` query pre-filtered to the current tenant.
    pub fn find<E: TenantScoped>(&self) -> Select<E> {
        E::find().filter(E::tenant_id_column().eq(self.tenant_id))
    }

    /// Return a `SELECT … WHERE id = ? AND tenant_id = ?` query.
    pub fn find_by_id<E, V>(&self, id: V) -> Select<E>
    where
        E: TenantScoped,
        V: Into<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>,
    {
        E::find_by_id(id).filter(E::tenant_id_column().eq(self.tenant_id))
    }

    /// Return an `UPDATE` query pre-filtered to the current tenant.
    pub fn update_many<E: TenantScoped>(&self) -> UpdateMany<E> {
        E::update_many().filter(E::tenant_id_column().eq(self.tenant_id))
    }

    /// Return a `DELETE` query pre-filtered to the current tenant.
    pub fn delete_many<E: TenantScoped>(&self) -> DeleteMany<E> {
        E::delete_many().filter(E::tenant_id_column().eq(self.tenant_id))
    }

    /// Construct a `TenantDb` directly from its components, for use in unit tests
    /// that cannot go through the Axum extractor machinery.
    #[cfg(test)]
    pub fn new_for_test(db: DatabaseConnection, tenant_id: Uuid) -> Self {
        Self { db, tenant_id }
    }
}

impl FromRequestParts<Arc<AppState>> for TenantDb {
    type Rejection = axum::response::Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let tenant = TenantContext::from_request_parts(parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(Self {
            db: state.db().clone(),
            tenant_id: tenant.tenant_id,
        })
    }
}
