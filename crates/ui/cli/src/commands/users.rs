use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::{CliError, Result};
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::roles::RoleResponse;
use uptrakit_openapi_client::types::users::{
    UpdateUserActiveRequest, UpdateUserRolesRequest, UserWithRolesResponse,
};

#[derive(Debug, Subcommand)]
pub enum UsersCommands {
    /// List all users with their roles
    List,
    /// Show user details including roles
    Show {
        /// User UUID
        id: Uuid,
    },
    /// Replace a user's roles
    #[command(group = clap::ArgGroup::new("roles_input").required(true))]
    SetRoles {
        /// User UUID
        id: Uuid,
        /// Role UUIDs to assign (replaces all existing roles)
        #[arg(group = "roles_input")]
        role_ids: Vec<Uuid>,
        /// Comma-separated role names to assign instead of UUIDs
        /// (resolved via the roles list; replaces all existing roles)
        #[arg(long, value_delimiter = ',', group = "roles_input")]
        names: Option<Vec<String>>,
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
        UsersCommands::SetRoles {
            id,
            role_ids,
            names,
        } => {
            let resp = if let Some(names) = names {
                set_roles_by_names(
                    &id,
                    &names,
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?
            } else {
                set_roles(
                    &id,
                    &role_ids,
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?
            };
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
        out
    }
}

impl HumanOutput for Vec<RoleResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No roles found.\n".to_string();
        }
        let mut out = format!("{:<38} {:<25} {:<10} TENANT\n", "ID", "NAME", "BUILT-IN");
        for r in self {
            out.push_str(&format!(
                "{:<38} {:<25} {:<10} {}\n",
                r.id,
                r.name,
                if r.is_built_in { "yes" } else { "no" },
                r.tenant_id
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "(global)".to_string())
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
        out.push_str(&format!(
            "Tenant:      {}\n",
            self.tenant_id
                .map(|t| t.to_string())
                .unwrap_or_else(|| "(global)".to_string())
        ));
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

/// Resolve role names to ids via the roles list, then replace the user's
/// roles through the same `update_user_roles` path as the UUID form.
pub async fn set_roles_by_names(
    id: &Uuid,
    names: &[String],
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<UserWithRolesResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let roles = client.list_roles().await.context_to()?;
    let mut role_ids = Vec::with_capacity(names.len());
    let mut unknown = Vec::new();
    for name in names {
        match roles.iter().find(|role| &role.name == name) {
            Some(role) => role_ids.push(role.id),
            None => unknown.push(name.clone()),
        }
    }
    if !unknown.is_empty() {
        return Err(report!(CliError::Other(format!(
            "unknown role name(s): {} (run `uptrakit roles list`)",
            unknown.join(", ")
        ))));
    }
    let req = UpdateUserRolesRequest { role_ids };
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
            tenant_id: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
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
}
