use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
pub use uptrakit_shared_types::UpdateStatus;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "update_history")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub software_item_id: Uuid,
    pub host_software_item_id: Option<Uuid>,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub status: UpdateStatus,
    #[sea_orm(column_type = "Text")]
    pub output: String,
    pub output_bytes: i64,
    pub actor_type: String,
    pub actor_id: String,
    pub execution_owner_service_id: Option<Uuid>,
    pub execution_owner_instance_id: Option<Uuid>,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    /// Classification of the update (security, bugfix, feature, unknown).
    #[sea_orm(default_value = "unknown")]
    pub update_category: String,
    /// Optional batch this update belongs to.
    pub batch_id: Option<Uuid>,
    /// Whether the update was dispatched in interactive mode (PTY allocated).
    ///
    /// Set at dispatch time and immutable — describes how the update was
    /// started. Used to show an "Input Required" badge in the history list
    /// for every in-progress interactive update.
    #[sea_orm(default_value = "false")]
    pub interactive: bool,
    /// Whether any output was dropped because the output size cap was exceeded.
    ///
    /// Set atomically on first truncation during the streaming phase. Persists
    /// across the update lifecycle so the history detail view can show a
    /// truncation warning even for completed updates.
    #[sea_orm(default_value = "false")]
    pub output_truncated: bool,
    /// Optional generic pre-update protection status.
    pub pre_update_protection_status: Option<String>,
    /// Optional human-readable summary of applied protection.
    pub pre_update_protection_summary: Option<String>,
    /// Optional recovery hint for failed/protected updates.
    pub recovery_hint: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
    #[sea_orm(
        belongs_to = "super::host::Entity",
        from = "Column::HostId",
        to = "super::host::Column::Id"
    )]
    Host,
    #[sea_orm(
        belongs_to = "super::software_item::Entity",
        from = "Column::SoftwareItemId",
        to = "super::software_item::Column::Id"
    )]
    SoftwareItem,
    #[sea_orm(
        belongs_to = "super::host_software_item::Entity",
        from = "Column::HostSoftwareItemId",
        to = "super::host_software_item::Column::Id"
    )]
    HostSoftwareItem,
    #[sea_orm(
        belongs_to = "super::update_batch::Entity",
        from = "Column::BatchId",
        to = "super::update_batch::Column::Id"
    )]
    UpdateBatch,
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Host.def()
    }
}

impl Related<super::software_item::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SoftwareItem.def()
    }
}

impl Related<super::host_software_item::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::HostSoftwareItem.def()
    }
}

impl Related<super::update_batch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UpdateBatch.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
