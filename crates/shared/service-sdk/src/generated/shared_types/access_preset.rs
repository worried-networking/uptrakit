// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::str::FromStr;
/// A predefined access preset that maps to a set of role names.
///
/// Presets are code-defined (not stored in DB) and provide quick role
/// bundles for user setup.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessPreset {
    /// Dashboard viewers, stakeholders — read-only access.
    ReadOnly,
    /// On-call staff — can trigger checks/updates, approve agents.
    Operator,
    /// Team leads — full CRUD on services, software, hosts.
    Manager,
    /// Tenant administrators — full tenant management.
    Administrator,
    /// System owner — full control including infrastructure.
    Owner,
}
impl AccessPreset {
    /// Returns the role names this preset assigns.
    pub fn roles(&self) -> &'static [&'static str] {
        match self {
            AccessPreset::ReadOnly => &["viewer"],
            AccessPreset::Operator => &["viewer", "operator"],
            AccessPreset::Manager => &[
                "viewer",
                "service_manager",
                "software_manager",
                "host_manager",
            ],
            AccessPreset::Administrator => &[
                "viewer",
                "service_manager",
                "software_manager",
                "host_manager",
                "settings_manager",
                "command_manager",
            ],
            AccessPreset::Owner => &[
                "viewer",
                "operator",
                "service_manager",
                "software_manager",
                "host_manager",
                "settings_manager",
                "command_manager",
                "system_administrator",
            ],
        }
    }
    /// Returns the canonical snake_case string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            AccessPreset::ReadOnly => "read_only",
            AccessPreset::Operator => "operator",
            AccessPreset::Manager => "manager",
            AccessPreset::Administrator => "administrator",
            AccessPreset::Owner => "owner",
        }
    }
    /// Returns a human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            AccessPreset::ReadOnly => "Dashboard viewers, stakeholders",
            AccessPreset::Operator => "On-call staff, trigger checks/updates, approve agents",
            AccessPreset::Manager => "Team leads with full CRUD on services, software, hosts",
            AccessPreset::Administrator => "Tenant administrators with full management",
            AccessPreset::Owner => "System owner with full control",
        }
    }
    /// Returns all available presets.
    pub fn all() -> &'static [AccessPreset] {
        &[
            AccessPreset::ReadOnly,
            AccessPreset::Operator,
            AccessPreset::Manager,
            AccessPreset::Administrator,
            AccessPreset::Owner,
        ]
    }
}
impl std::fmt::Display for AccessPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
/// Error returned when parsing an invalid [`AccessPreset`] string.
#[derive(Debug, thiserror::Error)]
#[error("invalid access preset value")]
pub struct ParseAccessPresetError;
impl FromStr for AccessPreset {
    type Err = ParseAccessPresetError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read_only" => Ok(Self::ReadOnly),
            "operator" => Ok(Self::Operator),
            "manager" => Ok(Self::Manager),
            "administrator" => Ok(Self::Administrator),
            "owner" => Ok(Self::Owner),
            _ => Err(ParseAccessPresetError),
        }
    }
}
