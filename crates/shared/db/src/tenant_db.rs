use crate::entity::TenantScoped;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DeleteMany, JoinType, PrimaryKeyTrait, QueryFilter,
    QuerySelect, RelationDef, Select, UpdateMany,
};
use uuid::Uuid;

/// Combined database connection + tenant identity.
///
/// Provides tenant-filtered query builders so that callers never accidentally
/// leak data across tenants. The Axum `FromRequestParts` extractor lives in
/// `uptrakit-web-api` and wraps this struct.
pub struct TenantDb {
    db: DatabaseConnection,
    pub tenant_id: Uuid,
}

impl TenantDb {
    /// Create a new `TenantDb` from its components.
    pub fn new(db: DatabaseConnection, tenant_id: Uuid) -> Self {
        Self { db, tenant_id }
    }

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

    /// Return a `SELECT` on `Target` joined to (and tenant-filtered through) a
    /// `TenantScoped` intermediate entity.
    ///
    /// Use for entities that have no `tenant_id` of their own but can be
    /// tenant-scoped by joining to a related `TenantScoped` entity.
    ///
    /// Example: `service_host` → `service` (TenantScoped):
    /// ```ignore
    /// tenant_db
    ///     .find_via_tenant_join::<service_host::Entity, service::Entity>(
    ///         service_host::Relation::Service.def(),
    ///     )
    ///     .filter(service::Column::DeactivatedAt.is_null())
    ///     .all(tenant_db.db())
    ///     .await?
    /// ```
    pub fn find_via_tenant_join<Target, Scoped>(&self, relation: RelationDef) -> Select<Target>
    where
        Target: sea_orm::EntityTrait,
        Scoped: TenantScoped,
    {
        Target::find()
            .join(JoinType::InnerJoin, relation)
            .filter(Scoped::tenant_id_column().eq(self.tenant_id))
    }
}
