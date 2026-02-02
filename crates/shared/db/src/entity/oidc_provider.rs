use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use time::OffsetDateTime;

/// Wrapper for `HashMap<String, String>` stored as JSON in the DB.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleMapping(pub HashMap<String, String>);

impl sea_orm::sea_query::ValueType for RoleMapping {
    fn try_from(v: sea_orm::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::Json(Some(json)) => {
                serde_json::from_value(*json).map_err(|_| sea_orm::sea_query::ValueTypeErr)
            }
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "RoleMapping".to_string()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::Json
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Json
    }
}

impl From<RoleMapping> for sea_orm::Value {
    fn from(val: RoleMapping) -> Self {
        let json = match serde_json::to_value(&val) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "RoleMapping serialization failed, using empty object");
                serde_json::Value::Object(Default::default())
            }
        };
        sea_orm::Value::Json(Some(Box::new(json)))
    }
}

impl sea_orm::TryGetable for RoleMapping {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        let json: serde_json::Value = res.try_get_by(index).map_err(sea_orm::TryGetError::DbErr)?;
        serde_json::from_value(json).map_err(|e| {
            sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                "failed to deserialize RoleMapping: {e}"
            )))
        })
    }
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "oidc_providers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: String,
    pub auto_create_users: bool,
    pub role_claim_path: Option<String>,
    #[sea_orm(column_type = "Json")]
    pub role_mapping: RoleMapping,
    pub is_active: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::user_oidc_link::Entity")]
    UserOidcLinks,
    #[sea_orm(has_many = "super::session::Entity")]
    Sessions,
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
}

impl Related<super::user_oidc_link::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UserOidcLinks.def()
    }
}

impl Related<super::session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Sessions.def()
    }
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
