use std::fmt;
use std::str::FromStr;

use sea_orm::entity::prelude::*;
use thiserror::Error;
use uptrakit_shared_db::crypto::EncryptedString;

// ── SshKeyType ──────────────────────────────────────────────────────

#[derive(Debug, Error)]
#[error("invalid SSH key type: expected ed25519, rsa, or ecdsa")]
pub struct ParseSshKeyTypeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshKeyType {
    Ed25519,
    Rsa,
    Ecdsa,
}

impl fmt::Display for SshKeyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl SshKeyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
            Self::Rsa => "rsa",
            Self::Ecdsa => "ecdsa",
        }
    }
}

impl FromStr for SshKeyType {
    type Err = ParseSshKeyTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ed25519" => Ok(Self::Ed25519),
            "rsa" => Ok(Self::Rsa),
            "ecdsa" => Ok(Self::Ecdsa),
            _ => Err(ParseSshKeyTypeError),
        }
    }
}

// ── SeaORM value conversions for SshKeyType ─────────────────────────

impl From<SshKeyType> for sea_orm::Value {
    fn from(val: SshKeyType) -> Self {
        sea_orm::Value::String(Some(val.as_str().to_string()))
    }
}

impl sea_orm::sea_query::ValueType for SshKeyType {
    fn try_from(v: sea_orm::Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::String(Some(s)) => s
                .parse::<SshKeyType>()
                .map_err(|_| sea_orm::sea_query::ValueTypeErr),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "SshKeyType".to_string()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Text
    }
}

impl sea_orm::sea_query::Nullable for SshKeyType {
    fn null() -> sea_orm::Value {
        sea_orm::Value::String(None)
    }
}

impl sea_orm::TryGetable for SshKeyType {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> std::result::Result<Self, sea_orm::TryGetError> {
        let s: String = res.try_get_by(index).map_err(sea_orm::TryGetError::DbErr)?;
        s.parse::<SshKeyType>().map_err(|e| {
            sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                "SshKeyType conversion failed: {e}"
            )))
        })
    }
}

// ── Entity ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "ssh_hosts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub port: i32,
    pub username: String,
    pub private_key: EncryptedString,
    pub key_type: SshKeyType,
    pub host_key_fingerprint: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_key_type_display() {
        assert_eq!(SshKeyType::Ed25519.to_string(), "ed25519");
        assert_eq!(SshKeyType::Rsa.to_string(), "rsa");
        assert_eq!(SshKeyType::Ecdsa.to_string(), "ecdsa");
    }

    #[test]
    fn ssh_key_type_from_str() {
        assert_eq!(
            "ed25519".parse::<SshKeyType>().expect("ok"),
            SshKeyType::Ed25519
        );
        assert_eq!("rsa".parse::<SshKeyType>().expect("ok"), SshKeyType::Rsa);
        assert_eq!(
            "ecdsa".parse::<SshKeyType>().expect("ok"),
            SshKeyType::Ecdsa
        );
    }

    #[test]
    fn ssh_key_type_from_str_invalid() {
        assert!("dsa".parse::<SshKeyType>().is_err());
        assert!("Ed25519".parse::<SshKeyType>().is_err());
        assert!("".parse::<SshKeyType>().is_err());
    }

    #[test]
    fn ssh_key_type_roundtrip_via_value() {
        for kt in [SshKeyType::Ed25519, SshKeyType::Rsa, SshKeyType::Ecdsa] {
            let val: sea_orm::Value = kt.into();
            let restored =
                <SshKeyType as sea_orm::sea_query::ValueType>::try_from(val).expect("roundtrip");
            assert_eq!(restored, kt);
        }
    }
}
