use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub hostname: String,
    pub friendly_name: String,
    pub ip_address: Option<String>,
    pub status: String,
    #[sea_orm(unique)]
    pub enrollment_secret_hash: String,
    pub agent_version: String,
    pub last_seen_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::agent_certificate::Entity")]
    AgentCertificate,
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
}

impl Related<super::agent_certificate::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::AgentCertificate.def()
    }
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl Related<super::host::Entity> for Entity {
    fn to() -> RelationDef {
        super::agent_host::Relation::Host.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::agent_host::Relation::Agent.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
