use serde::{Deserialize, Serialize};

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
}

impl Permission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::ViewSettings => "view_settings",
            Permission::ManageSettings => "manage_settings",
            Permission::ViewAgents => "view_agents",
            Permission::ManageAgents => "manage_agents",
            Permission::ManageGlobalSettings => "manage_global_settings",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "view_settings" => Some(Permission::ViewSettings),
            "manage_settings" => Some(Permission::ManageSettings),
            "view_agents" => Some(Permission::ViewAgents),
            "manage_agents" => Some(Permission::ManageAgents),
            "manage_global_settings" => Some(Permission::ManageGlobalSettings),
            _ => None,
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
