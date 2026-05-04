//! Generic per-table operations for the `db-migrate` subcommand.
//!
//! Plugins do not call these directly — they construct descriptors via
//! [`crate::PluginTableDescriptor::for_entity`], which captures `E` and
//! produces type-erased fn pointers wrapping these helpers.
//!
//! Each helper returns `Result<_, sea_orm::DbErr>` (no table name in
//! scope). The boundary helper (in `plugin-infrastructure-registry` or
//! `shared-db::migrate_core_tables`) wraps with `report!()` to attach
//! the table name as `TableMigrateError::Db { table, err }`.

#![cfg(feature = "migrations")]

use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, DatabaseConnection, DbErr, EntityTrait, IntoActiveModel,
    PaginatorTrait, QuerySelect,
};

pub(crate) async fn copy_one<E>(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64, DbErr>
where
    E: EntityTrait + 'static,
    E::Model: IntoActiveModel<E::ActiveModel> + Send + Sync + 'static,
    E::ActiveModel: ActiveModelTrait<Entity = E> + ActiveModelBehavior + Send + 'static,
{
    let mut copied = 0u64;
    let mut offset = 0u64;
    loop {
        let batch = E::find().offset(offset).limit(batch_size).all(src).await?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len() as u64;
        let active: Vec<_> = batch
            .into_iter()
            .map(IntoActiveModel::into_active_model)
            .collect();
        E::insert_many(active).exec(dst).await?;
        copied += n;
        offset += n;
    }
    Ok(copied)
}

pub(crate) async fn clean_one<E: EntityTrait>(dst: &DatabaseConnection) -> Result<(), DbErr> {
    E::delete_many().exec(dst).await.map(|_| ())
}

pub(crate) async fn verify_one<E>(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
) -> Result<(u64, u64), DbErr>
where
    E: EntityTrait + 'static,
    E::Model: Send + Sync + 'static,
{
    let src_count = E::find().count(src).await?;
    let dst_count = E::find().count(dst).await?;
    Ok((src_count, dst_count))
}
