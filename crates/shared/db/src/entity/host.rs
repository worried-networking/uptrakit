use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "hosts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub machine_id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub os_type: Option<String>,
    pub os_version: Option<String>,
    pub architecture: Option<String>,
    pub ip_address: Option<String>,
    pub last_seen_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl Related<super::agent::Entity> for Entity {
    fn to() -> RelationDef {
        super::agent_host::Relation::Agent.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::agent_host::Relation::Host.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
