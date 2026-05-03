use sea_orm::EntityTrait;

/// Marker trait for SeaORM entities scoped to a tenant via a `tenant_id` column.
///
/// Implementing this trait allows `TenantDb` to apply tenant-scoping filters automatically.
pub trait TenantScoped: EntityTrait {
    fn tenant_id_column() -> Self::Column;
}
