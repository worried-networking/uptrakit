use std::fmt;
use std::str::FromStr;

use sea_orm::entity::prelude::*;
use thiserror::Error;
use uptrakit_command::SudoContext;
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
    pub(crate) fn as_str(&self) -> &'static str {
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
    pub id: uuid::Uuid,
    pub name: String,
    pub hostname: String,
    pub port: i32,
    pub username: String,
    pub private_key: EncryptedString,
    pub key_type: SshKeyType,
    pub host_key_fingerprint: Option<String>,
    /// Machine ID of the remote host, populated from `ReportHosts` data.
    /// `None` until the host has been connected to at least once.
    pub machine_id: Option<String>,
    pub created_at: time::OffsetDateTime,
    pub updated_at: time::OffsetDateTime,
    /// Whether passwordless sudo (`sudo -n true`) is available for this host's agent user.
    ///
    /// `None` means the value has not yet been detected (host was bootstrapped
    /// before this column existed). `resolved_sudo_context()` defaults `None`
    /// to `true` for backward compatibility.
    pub sudo_available: Option<bool>,
    /// Whether the agent user is UID 0 (root) on the remote host.
    ///
    /// `None` means the value has not yet been detected.
    /// `resolved_sudo_context()` defaults `None` to `false`.
    pub is_root: Option<bool>,
    /// Sudo policy string: `"auto"` | `"force_with"` | `"force_without"`.
    ///
    /// Stored as TEXT with `DEFAULT 'auto'` so existing rows are valid without
    /// a data migration.
    pub sudo_policy: String,
    /// Controller-side plugin config ID for this PVE node's Proxmox plugin.
    ///
    /// Set after the controller confirms the `ReportPluginConfig` request.
    /// `None` for non-PVE hosts or before the config is reported.
    pub pve_plugin_config_id: Option<uuid::Uuid>,
    /// Short Proxmox VE node name (e.g. `"optiplex2"`).
    ///
    /// Collected from `hostname -s` during bootstrap or `host sync`.
    /// Used to match discovered guests to their PVE host node.
    /// `None` for non-PVE hosts or hosts not yet synced.
    pub pve_node_name: Option<String>,
}

impl Model {
    /// Build a [`SudoContext`] from the host's persisted sudo fields.
    ///
    /// Defaults for unknown (`None`) values:
    /// - `sudo_available`: `true` — backward compatibility for hosts bootstrapped
    ///   before this field was added (they had `NOPASSWD: ALL` written).
    /// - `is_root`: `false` — conservative default; the agent user is assumed
    ///   to be non-root until confirmed otherwise.
    /// - `sudo_policy`: `SudoPolicy::Auto` when the stored string cannot be
    ///   parsed.
    pub(crate) fn resolved_sudo_context(&self) -> SudoContext {
        SudoContext {
            is_root: self.is_root.unwrap_or(false),
            sudo_available: self.sudo_available.unwrap_or(true),
            policy: self.sudo_policy.parse().unwrap_or_default(),
        }
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use uptrakit_command::SudoPolicy;

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

    // ── resolved_sudo_context ────────────────────────────────────────────

    fn stub_model(sudo_available: Option<bool>, is_root: Option<bool>, sudo_policy: &str) -> Model {
        use uptrakit_crypto::{EncryptedString, init_master_key};
        // Ensure a test master key is set (no-op if already initialized).
        let _ = init_master_key(zeroize::Zeroizing::new([0x42u8; 32]));
        Model {
            id: uuid::Uuid::nil(),
            name: "test".to_string(),
            hostname: "127.0.0.1".to_string(),
            port: 22,
            username: "uptrakit".to_string(),
            private_key: EncryptedString::new("key".to_string(), "uptrakit:ssh_hosts:private_key")
                .expect("master key initialized above"),
            key_type: SshKeyType::Ed25519,
            host_key_fingerprint: None,
            machine_id: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
            sudo_available,
            is_root,
            sudo_policy: sudo_policy.to_string(),
            pve_plugin_config_id: None,
            pve_node_name: None,
        }
    }

    #[test]
    fn resolved_sudo_context_defaults_for_unknown_values() {
        let model = stub_model(None, None, "auto");
        let ctx = model.resolved_sudo_context();
        // Defaults: is_root=false, sudo_available=true, policy=Auto
        assert!(!ctx.is_root);
        assert!(ctx.sudo_available);
        assert_eq!(ctx.policy, SudoPolicy::Auto);
        // Should behave like old hardcoded sudo
        assert!(ctx.should_use_sudo());
    }

    #[test]
    fn resolved_sudo_context_root_user() {
        let model = stub_model(None, Some(true), "auto");
        let ctx = model.resolved_sudo_context();
        assert!(ctx.is_root);
        assert!(!ctx.should_use_sudo());
    }

    #[test]
    fn resolved_sudo_context_force_without_policy() {
        let model = stub_model(Some(true), Some(false), "force_without");
        let ctx = model.resolved_sudo_context();
        assert!(!ctx.should_use_sudo());
    }

    #[test]
    fn resolved_sudo_context_invalid_policy_defaults_to_auto() {
        let model = stub_model(Some(true), Some(false), "garbage_value");
        let ctx = model.resolved_sudo_context();
        assert_eq!(ctx.policy, SudoPolicy::Auto);
    }
}
