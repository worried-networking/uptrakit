use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum RevocationReason {
    #[sea_orm(string_value = "certificate_renewed")]
    CertificateRenewed,
    #[sea_orm(string_value = "agent_deactivated")]
    AgentDeactivated,
    #[sea_orm(string_value = "agent_merged")]
    AgentMerged,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_certificates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub serial_number: String,
    pub agent_id: Uuid,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revocation_reason: Option<RevocationReason>,
    pub created_at: OffsetDateTime,
    pub last_seen_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::agent::Entity",
        from = "Column::AgentId",
        to = "super::agent::Column::Id"
    )]
    Agent,
}

impl Related<super::agent::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Agent.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
