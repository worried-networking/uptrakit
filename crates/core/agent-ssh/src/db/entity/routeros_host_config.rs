use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "routeros_host_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub ssh_host_id: uuid::Uuid,
    /// Whether the `uptrakit` RouterOS group has the `reboot` policy.
    pub allow_reboot: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::ssh_host::Entity",
        from = "Column::SshHostId",
        to = "super::ssh_host::Column::Id",
        on_delete = "Cascade"
    )]
    SshHost,
}

impl ActiveModelBehavior for ActiveModel {}

impl Related<super::ssh_host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SshHost.def()
    }
}
