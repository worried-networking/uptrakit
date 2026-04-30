//! Authorization permissions used across shared surfaces, web API routes, and agents.

use std::str::FromStr;

/// An authorization permission.
///
/// Used in shared surface action descriptors (`SurfaceActionDescriptor.permission`) and
/// web API auth middleware to gate access to actions and endpoints.
///
/// All variants serialize to / deserialize from `snake_case` strings.
/// Unknown strings received on the wire are preserved as [`Permission::Other`] so that
/// old binaries do not silently lose permissions added by newer builds.
///
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, strum::EnumIter)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum Permission {
    // ── Services ─────────────────────────────────────────────────────────
    /// View tenant services and their status.
    ViewServices,
    /// Approve pending service enrollments.
    ApproveServices,
    /// Reject pending service enrollments.
    RejectServices,
    /// Deactivate/remove services.
    RemoveServices,
    /// Update service settings (ping interval, freeze, merge).
    UpdateServices,

    // ── System services ──────────────────────────────────────────────────
    /// View system services (MQTT bridge, external scheduler).
    ViewSystemServices,
    /// Approve pending system services.
    ApproveSystemServices,
    /// Reject pending system services.
    RejectSystemServices,
    /// Deactivate system services.
    RemoveSystemServices,
    /// Update system service settings.
    UpdateSystemServices,

    // ── Software ─────────────────────────────────────────────────────────
    /// View software items, plugin configs, history.
    ViewSoftware,
    /// Create software items and plugin configs.
    CreateSoftware,
    /// Edit software items and plugin configs.
    UpdateSoftware,
    /// Delete software items and plugin configs.
    DeleteSoftware,
    /// Trigger version checks and autodiscovery.
    TriggerChecks,
    /// Trigger update execution (single + batch).
    TriggerUpdates,
    /// Manage scheduled tasks.
    ManageScheduler,

    // ── Hosts ────────────────────────────────────────────────────────────
    /// View hosts.
    ViewHosts,
    /// Update host properties and tags.
    UpdateHosts,
    /// Deactivate hosts.
    DeactivateHosts,

    // ── Settings ─────────────────────────────────────────────────────────
    /// View all tenant settings (unified read).
    ViewSettings,
    /// Manage registration, authentication, OIDC providers.
    ManageAuthSettings,
    /// Manage tenant enrollment tokens.
    ManageEnrollmentTokens,
    /// Manage agent certificate settings.
    ManageAgentCerts,
    /// Manage global infrastructure settings.
    ManageGlobalSettings,

    // ── Commands ──────────────────────────────────────────────────────────
    /// Controls the ability to modify command-bearing plugin config fields
    /// (shell commands, Docker `post_pull_command`, and custom hook `commands`
    /// arrays). Granting this permission is equivalent to granting effective
    /// code-execution authority on all managed hosts assigned to the affected
    /// software items. Assign with the same care as granting root access.
    ManageCommands,

    // ── Notifications ────────────────────────────────────────────────────
    /// View notification channels, rules, log.
    ViewNotifications,
    /// Create/modify notification channels and rules; SMTP settings.
    ManageNotifications,

    // ── Audit logs ───────────────────────────────────────────────────────
    /// View tenant-scoped audit log entries.
    ViewAuditLogs,
    /// View system-level audit log entries.
    ViewSystemAuditLogs,

    // ── User management ──────────────────────────────────────────────────
    /// Manage user roles and access.
    ManageUsers,

    // ── Autodiscovery ────────────────────────────────────────────────────
    /// Manage autodiscovery ignore rules.
    ManageIgnores,

    // ── Plugin config testing ─────────────────────────────────────────
    /// Test plugin configurations against hosts (dry-run validation).
    TestPluginConfigs,

    // ── MCP ──────────────────────────────────────────────────────────────
    /// Access the MCP server endpoint (`/mcp`).
    ///
    /// Gate to the MCP endpoint. Tools enforce their own additional
    /// fine-grained permission checks (`ViewSoftware`, `TriggerUpdates`).
    AccessMcp,

    /// An unknown permission received from a newer build.
    ///
    /// Preserved on the wire instead of being dropped, so old binaries
    /// never silently lose permissions added in newer builds.
    #[strum(disabled)]
    Other(String),
}

impl Permission {
    /// Returns all permission variants.
    pub fn all() -> Vec<Self> {
        use strum::IntoEnumIterator;
        Self::iter().collect()
    }

    /// Returns the canonical `snake_case` string representation.
    pub fn as_str(&self) -> &str {
        match self {
            Permission::ViewServices => "view_services",
            Permission::ApproveServices => "approve_services",
            Permission::RejectServices => "reject_services",
            Permission::RemoveServices => "remove_services",
            Permission::UpdateServices => "update_services",
            Permission::ViewSystemServices => "view_system_services",
            Permission::ApproveSystemServices => "approve_system_services",
            Permission::RejectSystemServices => "reject_system_services",
            Permission::RemoveSystemServices => "remove_system_services",
            Permission::UpdateSystemServices => "update_system_services",
            Permission::ViewSoftware => "view_software",
            Permission::CreateSoftware => "create_software",
            Permission::UpdateSoftware => "update_software",
            Permission::DeleteSoftware => "delete_software",
            Permission::TriggerChecks => "trigger_checks",
            Permission::TriggerUpdates => "trigger_updates",
            Permission::ManageScheduler => "manage_scheduler",
            Permission::ViewHosts => "view_hosts",
            Permission::UpdateHosts => "update_hosts",
            Permission::DeactivateHosts => "deactivate_hosts",
            Permission::ViewSettings => "view_settings",
            Permission::ManageAuthSettings => "manage_auth_settings",
            Permission::ManageEnrollmentTokens => "manage_enrollment_tokens",
            Permission::ManageAgentCerts => "manage_agent_certs",
            Permission::ManageGlobalSettings => "manage_global_settings",
            Permission::ManageCommands => "manage_commands",
            Permission::ViewNotifications => "view_notifications",
            Permission::ManageNotifications => "manage_notifications",
            Permission::ViewAuditLogs => "view_audit_logs",
            Permission::ViewSystemAuditLogs => "view_system_audit_logs",
            Permission::ManageUsers => "manage_users",
            Permission::ManageIgnores => "manage_ignores",
            Permission::TestPluginConfigs => "test_plugin_configs",
            Permission::AccessMcp => "access_mcp",
            Permission::Other(s) => s.as_str(),
        }
    }

    /// Returns a human-readable description of the permission.
    pub fn description(&self) -> &str {
        match self {
            Permission::ViewServices => "View tenant services and their status",
            Permission::ApproveServices => "Approve pending service enrollments",
            Permission::RejectServices => "Reject pending service enrollments",
            Permission::RemoveServices => "Deactivate/remove services",
            Permission::UpdateServices => "Update service settings (ping interval, freeze, merge)",
            Permission::ViewSystemServices => {
                "View system services (MQTT bridge, external scheduler)"
            }
            Permission::ApproveSystemServices => "Approve pending system services",
            Permission::RejectSystemServices => "Reject pending system services",
            Permission::RemoveSystemServices => "Deactivate system services",
            Permission::UpdateSystemServices => "Update system service settings",
            Permission::ViewSoftware => "View software items, plugin configs, history",
            Permission::CreateSoftware => "Create software items and plugin configs",
            Permission::UpdateSoftware => "Edit software items and plugin configs",
            Permission::DeleteSoftware => "Delete software items and plugin configs",
            Permission::TriggerChecks => "Trigger version checks and autodiscovery",
            Permission::TriggerUpdates => "Trigger update execution (single and batch)",
            Permission::ManageScheduler => "Manage scheduled tasks",
            Permission::ViewHosts => "View hosts",
            Permission::UpdateHosts => "Update host properties and tags",
            Permission::DeactivateHosts => "Deactivate hosts",
            Permission::ViewSettings => "View all tenant settings",
            Permission::ManageAuthSettings => "Manage registration, authentication, OIDC providers",
            Permission::ManageEnrollmentTokens => "Manage tenant enrollment tokens",
            Permission::ManageAgentCerts => "Manage agent certificate settings",
            Permission::ManageGlobalSettings => "Manage global infrastructure settings",
            Permission::ManageCommands => {
                "Modify command-bearing plugin config fields (code execution authority)"
            }
            Permission::ViewNotifications => "View notification channels, rules, log",
            Permission::ManageNotifications => {
                "Create/modify notification channels and rules; SMTP settings"
            }
            Permission::ViewAuditLogs => "View tenant-scoped audit log entries",
            Permission::ViewSystemAuditLogs => "View system-level audit log entries",
            Permission::ManageUsers => "Manage user roles and access",
            Permission::ManageIgnores => "Manage autodiscovery ignore rules",
            Permission::TestPluginConfigs => "Test plugin configurations against hosts",
            Permission::AccessMcp => "Access the MCP server endpoint",
            Permission::Other(_) => "(unknown permission)",
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<Permission> for String {
    fn from(p: Permission) -> Self {
        p.as_str().to_string()
    }
}

impl From<String> for Permission {
    fn from(s: String) -> Self {
        match s.as_str() {
            "view_services" => Self::ViewServices,
            "approve_services" => Self::ApproveServices,
            "reject_services" => Self::RejectServices,
            "remove_services" => Self::RemoveServices,
            "update_services" => Self::UpdateServices,
            "view_system_services" => Self::ViewSystemServices,
            "approve_system_services" => Self::ApproveSystemServices,
            "reject_system_services" => Self::RejectSystemServices,
            "remove_system_services" => Self::RemoveSystemServices,
            "update_system_services" => Self::UpdateSystemServices,
            "view_software" => Self::ViewSoftware,
            "create_software" => Self::CreateSoftware,
            "update_software" => Self::UpdateSoftware,
            "delete_software" => Self::DeleteSoftware,
            "trigger_checks" => Self::TriggerChecks,
            "trigger_updates" => Self::TriggerUpdates,
            "manage_scheduler" => Self::ManageScheduler,
            "view_hosts" => Self::ViewHosts,
            "update_hosts" => Self::UpdateHosts,
            "deactivate_hosts" => Self::DeactivateHosts,
            "view_settings" => Self::ViewSettings,
            "manage_auth_settings" => Self::ManageAuthSettings,
            "manage_enrollment_tokens" => Self::ManageEnrollmentTokens,
            "manage_agent_certs" => Self::ManageAgentCerts,
            "manage_global_settings" => Self::ManageGlobalSettings,
            "manage_commands" => Self::ManageCommands,
            "view_notifications" => Self::ViewNotifications,
            "manage_notifications" => Self::ManageNotifications,
            "view_audit_logs" => Self::ViewAuditLogs,
            "view_system_audit_logs" => Self::ViewSystemAuditLogs,
            "manage_users" => Self::ManageUsers,
            "manage_ignores" => Self::ManageIgnores,
            "test_plugin_configs" => Self::TestPluginConfigs,
            "access_mcp" => Self::AccessMcp,
            _ => Self::Other(s),
        }
    }
}

impl FromStr for Permission {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s.to_string()))
    }
}

impl serde::Serialize for Permission {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for Permission {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Permission::from)
    }
}
