use serde::{Deserialize, Serialize};

/// A named role bundle (the demoted access-preset tiers): catalog metadata, never a server-side mechanism.
///
/// Presets are code-defined (not stored in DB) and provide quick role
/// bundles for user setup.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleBundle {
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

impl RoleBundle {
    /// Returns the role names this preset assigns.
    pub fn roles(&self) -> &'static [&'static str] {
        match self {
            RoleBundle::ReadOnly => &["viewer"],
            RoleBundle::Operator => &["viewer", "operator"],
            RoleBundle::Manager => &[
                "viewer",
                "service_manager",
                "software_manager",
                "host_manager",
            ],
            RoleBundle::Administrator => &[
                "viewer",
                "service_manager",
                "software_manager",
                "host_manager",
                "settings_manager",
                "command_manager",
            ],
            RoleBundle::Owner => &[
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
            RoleBundle::ReadOnly => "read_only",
            RoleBundle::Operator => "operator",
            RoleBundle::Manager => "manager",
            RoleBundle::Administrator => "administrator",
            RoleBundle::Owner => "owner",
        }
    }

    /// Returns a human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            RoleBundle::ReadOnly => "Dashboard viewers, stakeholders",
            RoleBundle::Operator => "On-call staff, trigger checks/updates, approve agents",
            RoleBundle::Manager => "Team leads with full CRUD on services, software, hosts",
            RoleBundle::Administrator => "Tenant administrators with full management",
            RoleBundle::Owner => "System owner with full control",
        }
    }

    /// Returns all available presets.
    pub fn all() -> &'static [RoleBundle] {
        &[
            RoleBundle::ReadOnly,
            RoleBundle::Operator,
            RoleBundle::Manager,
            RoleBundle::Administrator,
            RoleBundle::Owner,
        ]
    }
}

impl std::fmt::Display for RoleBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
