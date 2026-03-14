use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::access_presets::AccessPresetResponse;
use uptrakit_openapi_client::types::roles::RoleResponse;
use uptrakit_openapi_client::types::users::{
    ApplyPresetRequest, UpdateUserActiveRequest, UpdateUserRolesRequest, UserWithRolesResponse,
};

#[derive(Debug, Subcommand)]
pub enum UsersCommands {
    /// List all users with their roles
    List,
    /// Show user details including roles and permissions
    Show {
        /// User UUID
        id: Uuid,
    },
    /// Replace a user's roles
    SetRoles {
        /// User UUID
        id: Uuid,
        /// Role UUIDs to assign (replaces all existing roles)
        #[arg(required = true)]
        role_ids: Vec<Uuid>,
    },
    /// Activate a user
    Activate {
        /// User UUID
        id: Uuid,
    },
    /// Deactivate a user
    Deactivate {
        /// User UUID
        id: Uuid,
    },
    /// Apply an access preset to a user
    ApplyPreset {
        /// User UUID
        id: Uuid,
        /// Preset name to apply
        preset: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum RolesCommands {
    /// List all roles with their permissions
    List,
    /// Show role details including permissions
    Show {
        /// Role UUID
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
pub enum AccessPresetsCommands {
    /// List all access presets
    List,
}

pub async fn dispatch_users(command: UsersCommands, ctx: &CliContext) -> Result<()> {
    match command {
        UsersCommands::List => {
            let resp = list(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        UsersCommands::Show { id } => {
            let resp = show(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        UsersCommands::SetRoles { id, role_ids } => {
            let resp = set_roles(
                &id,
                &role_ids,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        UsersCommands::Activate { id } => {
            let resp = set_active(
                &id,
                true,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        UsersCommands::Deactivate { id } => {
            let resp = set_active(
                &id,
                false,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        UsersCommands::ApplyPreset { id, preset } => {
            let resp = apply_preset(
                &id,
                &preset,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

pub async fn dispatch_roles(command: RolesCommands, ctx: &CliContext) -> Result<()> {
    match command {
        RolesCommands::List => {
            let resp = list_roles(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        RolesCommands::Show { id } => {
            let resp = show_role(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

pub async fn dispatch_access_presets(
    command: AccessPresetsCommands,
    ctx: &CliContext,
) -> Result<()> {
    match command {
        AccessPresetsCommands::List => {
            let resp = list_presets(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
    }
    Ok(())
}

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for Vec<UserWithRolesResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No users found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<30} {:<20} {:<8} ROLES\n",
            "ID", "EMAIL", "NAME", "ACTIVE"
        );
        for u in self {
            let name = format!("{} {}", u.first_name, u.last_name);
            let roles: Vec<&str> = u.roles.iter().map(|r| r.name.as_str()).collect();
            out.push_str(&format!(
                "{:<38} {:<30} {:<20} {:<8} {}\n",
                u.id,
                u.email,
                name,
                if u.is_active { "yes" } else { "no" },
                roles.join(", ")
            ));
        }
        out
    }
}

impl HumanOutput for UserWithRolesResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:          {}\n", self.id));
        out.push_str(&format!("Email:       {}\n", self.email));
        out.push_str(&format!(
            "Name:        {} {}\n",
            self.first_name, self.last_name
        ));
        out.push_str(&format!(
            "Active:      {}\n",
            if self.is_active { "yes" } else { "no" }
        ));
        if self.roles.is_empty() {
            out.push_str("Roles:       (none)\n");
        } else {
            out.push_str("Roles:\n");
            for r in &self.roles {
                out.push_str(&format!("  - {} ({})\n", r.name, r.id));
            }
        }
        if self.permissions.is_empty() {
            out.push_str("Permissions: (none)\n");
        } else {
            out.push_str("Permissions:\n");
            for p in &self.permissions {
                out.push_str(&format!("  - {p}\n"));
            }
        }
        out
    }
}

impl HumanOutput for Vec<RoleResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No roles found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<25} {:<10} PERMISSIONS\n",
            "ID", "NAME", "BUILT-IN"
        );
        for r in self {
            let perms: Vec<String> = r.permissions.iter().map(|p| p.to_string()).collect();
            out.push_str(&format!(
                "{:<38} {:<25} {:<10} {}\n",
                r.id,
                r.name,
                if r.is_built_in { "yes" } else { "no" },
                perms.join(", ")
            ));
        }
        out
    }
}

impl HumanOutput for RoleResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:          {}\n", self.id));
        out.push_str(&format!("Name:        {}\n", self.name));
        if let Some(ref desc) = self.description {
            out.push_str(&format!("Description: {desc}\n"));
        }
        out.push_str(&format!(
            "Built-in:    {}\n",
            if self.is_built_in { "yes" } else { "no" }
        ));
        if self.permissions.is_empty() {
            out.push_str("Permissions: (none)\n");
        } else {
            out.push_str("Permissions:\n");
            for p in &self.permissions {
                out.push_str(&format!("  - {p}\n"));
            }
        }
        out
    }
}

impl HumanOutput for Vec<AccessPresetResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No access presets found.\n".to_string();
        }
        let mut out = format!("{:<25} {:<40} ROLES\n", "NAME", "DESCRIPTION");
        for p in self {
            out.push_str(&format!(
                "{:<25} {:<40} {}\n",
                p.name,
                p.description,
                p.roles.join(", ")
            ));
        }
        out
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all users with roles.
pub async fn list(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<Vec<UserWithRolesResponse>> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.list_users().await.context_to()
}

/// Show details for a single user.
pub async fn show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UserWithRolesResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_user(id).await.context_to()
}

/// Replace a user's roles.
pub async fn set_roles(
    id: &Uuid,
    role_ids: &[Uuid],
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UserWithRolesResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateUserRolesRequest {
        role_ids: role_ids.to_vec(),
    };
    client.update_user_roles(id, &req).await.context_to()
}

/// Activate or deactivate a user.
pub async fn set_active(
    id: &Uuid,
    is_active: bool,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UserWithRolesResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = UpdateUserActiveRequest { is_active };
    client.update_user_active(id, &req).await.context_to()
}

/// Apply an access preset to a user.
pub async fn apply_preset(
    id: &Uuid,
    preset: &str,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UserWithRolesResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = ApplyPresetRequest {
        preset: preset.to_string(),
    };
    client.apply_preset(id, &req).await.context_to()
}

/// List all roles.
pub async fn list_roles(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<Vec<RoleResponse>> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.list_roles().await.context_to()
}

/// Show details for a single role.
pub async fn show_role(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<RoleResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_role(id).await.context_to()
}

/// List all access presets.
pub async fn list_presets(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<Vec<AccessPresetResponse>> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.list_access_presets().await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_openapi_client::types::users::UserRoleSummary;

    fn sample_user() -> UserWithRolesResponse {
        UserWithRolesResponse {
            id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                .parse::<Uuid>()
                .unwrap(),
            email: "user@example.com".to_string(),
            first_name: "Jane".to_string(),
            last_name: "Doe".to_string(),
            is_active: true,
            roles: vec![UserRoleSummary {
                id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                    .parse::<Uuid>()
                    .unwrap(),
                name: "admin".to_string(),
            }],
            permissions: vec![],
        }
    }

    #[test]
    fn user_detail_human_output() {
        let u = sample_user();
        let s = u.to_human_string();
        assert!(s.contains("user@example.com"), "email missing");
        assert!(s.contains("Jane Doe"), "name missing");
        assert!(s.contains("admin"), "role missing");
        assert!(s.contains("yes"), "active status missing");
    }

    #[test]
    fn user_list_empty() {
        let users: Vec<UserWithRolesResponse> = vec![];
        assert!(users.to_human_string().contains("No users found"));
    }

    #[test]
    fn user_list_has_rows() {
        let users = vec![sample_user()];
        let s = users.to_human_string();
        assert!(s.contains("user@example.com"), "email missing");
    }

    fn sample_role() -> RoleResponse {
        RoleResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            name: "admin".to_string(),
            description: Some("Full administrator access".to_string()),
            is_built_in: true,
            permissions: vec![],
        }
    }

    #[test]
    fn role_detail_human_output() {
        let r = sample_role();
        let s = r.to_human_string();
        assert!(s.contains("admin"), "name missing");
        assert!(
            s.contains("Full administrator access"),
            "description missing"
        );
        assert!(s.contains("yes"), "built-in flag missing");
    }

    #[test]
    fn role_list_empty() {
        let roles: Vec<RoleResponse> = vec![];
        assert!(roles.to_human_string().contains("No roles found"));
    }

    #[test]
    fn preset_list_empty() {
        let presets: Vec<AccessPresetResponse> = vec![];
        assert!(
            presets
                .to_human_string()
                .contains("No access presets found")
        );
    }

    #[test]
    fn preset_list_has_rows() {
        let presets = vec![AccessPresetResponse {
            name: "viewer".to_string(),
            description: "Read-only access".to_string(),
            roles: vec!["viewer".to_string()],
        }];
        let s = presets.to_human_string();
        assert!(s.contains("viewer"), "name missing");
        assert!(s.contains("Read-only access"), "description missing");
    }
}
