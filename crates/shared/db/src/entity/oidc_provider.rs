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
        // `HashMap<String, String>` serialization to JSON is infallible: both key
        // and value types are JSON-native strings with no custom serializers, so
        // `serde_json::to_value` cannot fail for this type. The fallback exists
        // only as a defence-in-depth safeguard against future type changes; it
        // writes a clearly invalid sentinel instead of a silent empty `{}` that
        // could be confused with a legitimate empty mapping.
        let json = match serde_json::to_value(&val) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "RoleMapping serialization failed — writing sentinel error value"
                );
                serde_json::json!({"__serialization_error": true})
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
    pub client_secret: crate::crypto::EncryptedString,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_mapping_serialization_round_trip() {
        let mut map = HashMap::new();
        map.insert("admin_group".to_string(), "admin".to_string());
        map.insert("viewer_group".to_string(), "viewer".to_string());
        let role_mapping = RoleMapping(map.clone());

        let value: sea_orm::Value = role_mapping.into();
        match value {
            sea_orm::Value::Json(Some(json)) => {
                let deserialized: RoleMapping =
                    serde_json::from_value(*json).expect("should deserialize");
                assert_eq!(deserialized.0, map);
            }
            other => panic!("expected Json value, got: {other:?}"),
        }
    }

    #[test]
    fn role_mapping_empty_serialization() {
        let role_mapping = RoleMapping(HashMap::new());

        let value: sea_orm::Value = role_mapping.into();
        match value {
            sea_orm::Value::Json(Some(json)) => {
                assert_eq!(*json, serde_json::json!({}));
            }
            other => panic!("expected Json value, got: {other:?}"),
        }
    }

    #[test]
    fn role_mapping_serde_json_round_trip() {
        let mut map = HashMap::new();
        map.insert("group_a".to_string(), "role_x".to_string());
        let role_mapping = RoleMapping(map.clone());

        let json = serde_json::to_value(&role_mapping).expect("serialization is infallible");
        let deserialized: RoleMapping = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(deserialized.0, map);
    }
}
