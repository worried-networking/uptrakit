use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(test, derive(strum::EnumIter))]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ViewSettings,
    ManageSettings,
    ViewAgents,
    ManageAgents,
    ManageGlobalSettings,
    ViewSoftware,
    ManageSoftware,
    ViewHosts,
    ManageHosts,
    ViewNotifications,
    ManageNotifications,
}

impl Permission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::ViewSettings => "view_settings",
            Permission::ManageSettings => "manage_settings",
            Permission::ViewAgents => "view_agents",
            Permission::ManageAgents => "manage_agents",
            Permission::ManageGlobalSettings => "manage_global_settings",
            Permission::ViewSoftware => "view_software",
            Permission::ManageSoftware => "manage_software",
            Permission::ViewHosts => "view_hosts",
            Permission::ManageHosts => "manage_hosts",
            Permission::ViewNotifications => "view_notifications",
            Permission::ManageNotifications => "manage_notifications",
        }
    }
}

/// Error returned when parsing an invalid [`Permission`] string.
#[derive(Debug, Error)]
#[error("invalid permission value")]
pub struct ParsePermissionError;

impl FromStr for Permission {
    type Err = ParsePermissionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "view_settings" => Ok(Self::ViewSettings),
            "manage_settings" => Ok(Self::ManageSettings),
            "view_agents" => Ok(Self::ViewAgents),
            "manage_agents" => Ok(Self::ManageAgents),
            "manage_global_settings" => Ok(Self::ManageGlobalSettings),
            "view_software" => Ok(Self::ViewSoftware),
            "manage_software" => Ok(Self::ManageSoftware),
            "view_hosts" => Ok(Self::ViewHosts),
            "manage_hosts" => Ok(Self::ManageHosts),
            "view_notifications" => Ok(Self::ViewNotifications),
            "manage_notifications" => Ok(Self::ManageNotifications),
            _ => Err(ParsePermissionError),
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
