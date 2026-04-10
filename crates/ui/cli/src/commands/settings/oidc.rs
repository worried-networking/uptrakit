use std::collections::HashMap;

use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::SecretString;
use uptrakit_openapi_client::types::oidc_providers::{
    CreateOidcProviderRequest, OidcProviderResponse, UpdateOidcProviderRequest,
};

use super::DeletedOutput;

#[derive(Debug, Subcommand)]
pub enum OidcCommands {
    /// List OIDC providers
    List,
    /// Show OIDC provider details
    Show {
        /// OIDC provider UUID
        id: Uuid,
    },
    /// Create a new OIDC provider
    Create {
        /// Provider display name
        #[arg(long)]
        name: String,
        /// URL-safe slug
        #[arg(long)]
        slug: String,
        /// Logo URL
        #[arg(long)]
        logo_url: Option<String>,
        /// OIDC issuer URL
        #[arg(long)]
        issuer_url: String,
        /// OAuth client ID
        #[arg(long)]
        client_id: String,
        /// OAuth client secret
        #[arg(long)]
        client_secret: String,
        /// OAuth scopes (default: "openid email profile groups")
        #[arg(long)]
        scopes: Option<String>,
        /// Auto-create users on first login
        #[arg(long)]
        auto_create_users: Option<bool>,
        /// Whether issuer hostnames may resolve to private-network addresses
        #[arg(long)]
        allow_private_network_issuers: Option<bool>,
        /// JSONPath for role claim
        #[arg(long)]
        role_claim_path: Option<String>,
    },
    /// Update an OIDC provider
    Update {
        /// OIDC provider UUID
        id: Uuid,
        /// Provider display name
        #[arg(long)]
        name: Option<String>,
        /// URL-safe slug
        #[arg(long)]
        slug: Option<String>,
        /// Logo URL
        #[arg(long)]
        logo_url: Option<String>,
        /// OIDC issuer URL
        #[arg(long)]
        issuer_url: Option<String>,
        /// OAuth client ID
        #[arg(long)]
        client_id: Option<String>,
        /// OAuth client secret
        #[arg(long)]
        client_secret: Option<String>,
        /// OAuth scopes
        #[arg(long)]
        scopes: Option<String>,
        /// Auto-create users on first login
        #[arg(long)]
        auto_create_users: Option<bool>,
        /// Whether issuer hostnames may resolve to private-network addresses
        #[arg(long)]
        allow_private_network_issuers: Option<bool>,
        /// JSONPath for role claim
        #[arg(long)]
        role_claim_path: Option<String>,
    },
    /// Delete an OIDC provider
    Delete {
        /// OIDC provider UUID
        id: Uuid,
    },
    /// Activate an OIDC provider
    Activate {
        /// OIDC provider UUID
        id: Uuid,
    },
    /// Deactivate an OIDC provider
    Deactivate {
        /// OIDC provider UUID
        id: Uuid,
    },
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for creating an OIDC provider.
pub struct OidcCreateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Option<String>,
    pub auto_create_users: Option<bool>,
    pub allow_private_network_issuers: Option<bool>,
    pub role_claim_path: Option<String>,
}

/// Parameters for updating an OIDC provider.
pub struct OidcUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub id: Uuid,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub logo_url: Option<String>,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scopes: Option<String>,
    pub auto_create_users: Option<bool>,
    pub allow_private_network_issuers: Option<bool>,
    pub role_claim_path: Option<String>,
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for Vec<OidcProviderResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No OIDC providers found.\n".to_string();
        }
        let mut out = format!("{:<38} {:<20} {:<15} ACTIVE\n", "ID", "NAME", "SLUG");
        for p in self {
            out.push_str(&format!(
                "{:<38} {:<20} {:<15} {}\n",
                p.id, p.name, p.slug, p.is_active
            ));
        }
        out
    }
}

impl HumanOutput for OidcProviderResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:               {}\n", self.id));
        out.push_str(&format!("Name:             {}\n", self.name));
        out.push_str(&format!("Slug:             {}\n", self.slug));
        if let Some(ref logo) = self.logo_url {
            out.push_str(&format!("Logo URL:         {}\n", logo));
        }
        out.push_str(&format!("Issuer URL:       {}\n", self.issuer_url));
        out.push_str(&format!("Client ID:        {}\n", self.client_id));
        out.push_str(&format!("Has Secret:       {}\n", self.has_client_secret));
        out.push_str(&format!("Scopes:           {}\n", self.scopes));
        out.push_str(&format!("Auto Create Users: {}\n", self.auto_create_users));
        out.push_str(&format!(
            "Allow Private Network Issuers: {}\n",
            self.allow_private_network_issuers
        ));
        if let Some(ref path) = self.role_claim_path {
            out.push_str(&format!("Role Claim Path:  {}\n", path));
        }
        if !self.role_mapping.is_empty() {
            let mappings: Vec<String> = self
                .role_mapping
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            out.push_str(&format!("Role Mapping:     {}\n", mappings.join(", ")));
        }
        out.push_str(&format!("Active:           {}\n", self.is_active));
        out.push_str(&format!("Created:          {}\n", self.created_at));
        out.push_str(&format!("Updated:          {}\n", self.updated_at));
        out
    }
}

// ── Dispatch ─────────────────────────────────────────────────────────────────

pub async fn dispatch_oidc(command: OidcCommands, ctx: &CliContext) -> Result<()> {
    match command {
        OidcCommands::List => {
            let resp = oidc_list(
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Show { id } => {
            let resp = oidc_show(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Create {
            name,
            slug,
            logo_url,
            issuer_url,
            client_id,
            client_secret,
            scopes,
            auto_create_users,
            allow_private_network_issuers,
            role_claim_path,
        } => {
            let resp = oidc_create(OidcCreateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                name,
                slug,
                logo_url,
                issuer_url,
                client_id,
                client_secret,
                scopes,
                auto_create_users,
                allow_private_network_issuers,
                role_claim_path,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Update {
            id,
            name,
            slug,
            logo_url,
            issuer_url,
            client_id,
            client_secret,
            scopes,
            auto_create_users,
            allow_private_network_issuers,
            role_claim_path,
        } => {
            let resp = oidc_update(OidcUpdateParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                id,
                name,
                slug,
                logo_url,
                issuer_url,
                client_id,
                client_secret,
                scopes,
                auto_create_users,
                allow_private_network_issuers,
                role_claim_path,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Delete { id } => {
            let resp = oidc_delete(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Activate { id } => {
            let resp = oidc_activate(
                &id,
                ctx.server.as_deref(),
                ctx.token.as_deref(),
                ctx.insecure,
                ctx.request_timeout,
            )
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        OidcCommands::Deactivate { id } => {
            let resp = oidc_deactivate(
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

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all OIDC providers.
pub async fn oidc_list(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<Vec<OidcProviderResponse>> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.list_oidc_providers().await.context_to()
}

/// Show a single OIDC provider.
pub async fn oidc_show(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<OidcProviderResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_oidc_provider(id).await.context_to()
}

/// Create a new OIDC provider.
pub async fn oidc_create(params: OidcCreateParams<'_>) -> Result<OidcProviderResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateOidcProviderRequest {
        name: params.name,
        slug: params.slug,
        logo_url: params.logo_url,
        issuer_url: params.issuer_url,
        client_id: params.client_id,
        client_secret: SecretString::new(params.client_secret),
        scopes: params
            .scopes
            .unwrap_or_else(|| "openid email profile groups".to_string()),
        auto_create_users: params.auto_create_users.unwrap_or(true),
        allow_private_network_issuers: params.allow_private_network_issuers,
        role_claim_path: params.role_claim_path,
        role_mapping: HashMap::new(),
    };
    client.create_oidc_provider(&req).await.context_to()
}

/// Update an existing OIDC provider.
pub async fn oidc_update(params: OidcUpdateParams<'_>) -> Result<OidcProviderResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateOidcProviderRequest {
        name: params.name,
        slug: params.slug,
        logo_url: params.logo_url,
        issuer_url: params.issuer_url,
        client_id: params.client_id,
        client_secret: params.client_secret.map(SecretString::new),
        scopes: params.scopes,
        auto_create_users: params.auto_create_users,
        allow_private_network_issuers: params.allow_private_network_issuers,
        role_claim_path: params.role_claim_path,
        role_mapping: None,
    };
    client
        .update_oidc_provider(&params.id, &req)
        .await
        .context_to()
}

/// Delete an OIDC provider.
pub async fn oidc_delete(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<DeletedOutput> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.delete_oidc_provider(id).await.context_to()?;
    Ok(DeletedOutput {
        message: format!("OIDC provider {id} deleted."),
    })
}

/// Activate an OIDC provider.
pub async fn oidc_activate(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<OidcProviderResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.activate_oidc_provider(id).await.context_to()
}

/// Deactivate an OIDC provider.
pub async fn oidc_deactivate(
    id: &Uuid,
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<OidcProviderResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.deactivate_oidc_provider(id).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oidc_list_empty() {
        let resp: Vec<OidcProviderResponse> = vec![];
        assert!(resp.to_human_string().contains("No OIDC"));
    }
}
