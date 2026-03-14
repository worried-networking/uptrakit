use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::commands::settings::DeletedOutput;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;

#[derive(Debug, Subcommand)]
pub enum AutodiscoveryCommands {
    /// Manage autodiscovery ignore rules
    Ignores {
        #[command(subcommand)]
        command: IgnoresCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum IgnoresCommands {
    /// List autodiscovery ignore rules
    List {
        /// Page number
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
    },
    /// Create an autodiscovery ignore rule
    Create {
        /// Software item name to suppress from future discoveries
        #[arg(long)]
        name: String,
    },
    /// Delete an autodiscovery ignore rule
    Delete {
        /// Ignore rule UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Perform a batch action on multiple autodiscovery ignore rules
    Batch {
        /// Action to perform (e.g. delete)
        action: String,
        /// Ignore rule UUIDs (space-separated)
        ids: Vec<uptrakit_openapi_client::Uuid>,
    },
}

pub async fn dispatch(command: AutodiscoveryCommands, ctx: &CliContext) -> Result<()> {
    match command {
        AutodiscoveryCommands::Ignores { command } => match command {
            IgnoresCommands::List { page, per_page } => {
                let resp = ignores_list(IgnoresListParams {
                    server: ctx.server.as_deref(),
                    token: ctx.token.as_deref(),
                    insecure: ctx.insecure,
                    page,
                    per_page,
                    request_timeout: ctx.request_timeout,
                })
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            IgnoresCommands::Create { name } => {
                let resp = ignores_create(IgnoresCreateParams {
                    name,
                    server: ctx.server.as_deref(),
                    token: ctx.token.as_deref(),
                    insecure: ctx.insecure,
                    request_timeout: ctx.request_timeout,
                })
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            IgnoresCommands::Delete { id } => {
                let resp = ignores_delete(IgnoresDeleteParams {
                    id: &id,
                    server: ctx.server.as_deref(),
                    token: ctx.token.as_deref(),
                    insecure: ctx.insecure,
                    request_timeout: ctx.request_timeout,
                })
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
            IgnoresCommands::Batch { action, ids } => {
                let resp = batch(
                    &action,
                    &ids,
                    ctx.server.as_deref(),
                    ctx.token.as_deref(),
                    ctx.insecure,
                    ctx.request_timeout,
                )
                .await?;
                crate::output::print_output(ctx.format, &resp)?;
            }
        },
    }
    Ok(())
}
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::autodiscovery::ListIgnoresParams;
use uptrakit_openapi_client::types::autodiscovery::{
    CreateSoftwareIgnoreRequest, SoftwareIgnoreResponse,
};
use uptrakit_openapi_client::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<SoftwareIgnoreResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No software ignore rules found.\n".to_string();
        }
        let mut out = format!("{:<38} NAME\n", "ID");
        for r in &self.items {
            out.push_str(&format!("{:<38} {}\n", r.id, r.name));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for SoftwareIgnoreResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:               {}\n", self.id));
        out.push_str(&format!("Name:             {}\n", self.name));
        out.push_str(&format!(
            "Created:          {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

pub struct IgnoresListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

pub struct IgnoresCreateParams<'a> {
    pub name: String,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct IgnoresDeleteParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

pub async fn ignores_list(
    params: IgnoresListParams<'_>,
) -> Result<PaginatedResponse<SoftwareIgnoreResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let list_params = ListIgnoresParams {
        page: params.page,
        per_page: params.per_page,
    };
    client
        .list_software_ignores(&list_params)
        .await
        .context_to()
}

pub async fn ignores_create(params: IgnoresCreateParams<'_>) -> Result<SoftwareIgnoreResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateSoftwareIgnoreRequest {
        name: params.name,
        host_id: None,
    };
    client.create_software_ignore(&req).await.context_to()
}

pub async fn ignores_delete(params: IgnoresDeleteParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .delete_software_ignore(params.id)
        .await
        .context_to()?;
    Ok(DeletedOutput {
        message: format!("Software ignore rule {} deleted.", params.id),
    })
}

/// Perform a batch action on multiple software ignore rules.
pub async fn batch(
    action: &str,
    ids: &[Uuid],
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<BatchActionResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    let req = BatchActionRequest {
        action: action.to_string(),
        ids: ids.to_vec(),
    };
    client.batch_software_ignores(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_ignore() -> SoftwareIgnoreResponse {
        SoftwareIgnoreResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            name: "FreshRSS".to_string(),
            host_id: None,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    #[test]
    fn ignore_detail_human_output() {
        let r = sample_ignore();
        let s = r.to_human_string();
        assert!(s.contains("FreshRSS"), "name missing");
    }

    #[test]
    fn paginated_ignores_empty() {
        let resp: PaginatedResponse<SoftwareIgnoreResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No software ignore"));
    }

    #[test]
    fn paginated_ignores_has_row() {
        let resp = PaginatedResponse {
            items: vec![sample_ignore()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("FreshRSS"), "name missing");
    }
}
