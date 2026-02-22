use std::fmt;
use std::str::FromStr;

use sea_orm::entity::prelude::*;
use thiserror::Error;
use uptrakit_crypto::EncryptedString;

// ── SshKeyType ──────────────────────────────────────────────────────

#[derive(Debug, Error)]
#[error("invalid SSH key type: expected ed25519, rsa, or ecdsa")]
pub struct ParseSshKeyTypeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, sea_orm::DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum SshKeyType {
    #[sea_orm(string_value = "ed25519")]
    Ed25519,
    #[sea_orm(string_value = "rsa")]
    Rsa,
    #[sea_orm(string_value = "ecdsa")]
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
    /// Machine ID of the remote host, populated from `ReportHosts` data.
    /// Empty string until the host has been connected to at least once.
    pub machine_id: String,
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
}
