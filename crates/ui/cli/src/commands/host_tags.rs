use crate::client::authenticated_client;
use crate::commands::CliContext;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Subcommand)]
pub enum HostTagsCommands {
    /// List all host tags
    List {
        /// Page number (1-indexed)
        #[arg(long)]
        page: Option<u64>,
        /// Items per page
        #[arg(long)]
        per_page: Option<u64>,
        /// Search by name
        #[arg(long)]
        search: Option<String>,
    },
    /// Show host tag details
    Show {
        /// Host tag UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Create a new host tag
    Create {
        /// Tag name
        #[arg(long)]
        name: String,
        /// Tag color (hex, e.g. #3B82F6). Auto-generated if omitted.
        #[arg(long)]
        color: Option<String>,
        /// Tag description
        #[arg(long)]
        description: Option<String>,
    },
    /// Update a host tag
    Update {
        /// Host tag UUID
        id: uptrakit_openapi_client::Uuid,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New color (hex, e.g. #3B82F6)
        #[arg(long)]
        color: Option<String>,
        /// New description
        #[arg(long)]
        description: Option<String>,
        /// Clear the description
        #[arg(long)]
        clear_description: bool,
    },
    /// Delete a host tag
    Delete {
        /// Host tag UUID
        id: uptrakit_openapi_client::Uuid,
    },
    /// Set tags on a host (replaces existing tags)
    Set {
        /// Host UUID
        host_id: uptrakit_openapi_client::Uuid,
        /// Tag UUIDs (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<uptrakit_openapi_client::Uuid>,
    },
    /// Perform a batch action on multiple host tags
    Batch {
        /// Action to perform (e.g. delete)
        action: String,
        /// Host tag UUIDs (space-separated)
        ids: Vec<uptrakit_openapi_client::Uuid>,
    },
}

pub async fn dispatch(command: HostTagsCommands, ctx: &CliContext) -> Result<()> {
    match command {
        HostTagsCommands::List {
            page,
            per_page,
            search,
        } => {
            let resp = list(ListParams {
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
                page,
                per_page,
                search,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        HostTagsCommands::Show { id } => {
            let resp = show(ShowParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        HostTagsCommands::Create {
            name,
            color,
            description,
        } => {
            let resp = create(CreateParams {
                name,
                color,
                description,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        HostTagsCommands::Update {
            id,
            name,
            color,
            description,
            clear_description,
        } => {
            let resp = update(UpdateParams {
                id: &id,
                name,
                color,
                description,
                clear_description,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        HostTagsCommands::Delete { id } => {
            let resp = delete(DeleteParams {
                id: &id,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        HostTagsCommands::Set { host_id, tags } => {
            let resp = set_tags(SetParams {
                host_id: &host_id,
                tag_ids: tags,
                server: ctx.server.as_deref(),
                token: ctx.token.as_deref(),
                insecure: ctx.insecure,
                request_timeout: ctx.request_timeout,
            })
            .await?;
            crate::output::print_output(ctx.format, &resp)?;
        }
        HostTagsCommands::Batch { action, ids } => {
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
    }
    Ok(())
}
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_openapi_client::types::host_tags::{
    CreateHostTagRequest, HostTagResponse, HostTagSummary, ListHostTagsQuery, SetHostTagsRequest,
    UpdateHostTagRequest,
};
use uptrakit_openapi_client::types::hosts::HostMessageResponse;
use uptrakit_openapi_client::types::pagination::PaginatedResponse;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<HostTagResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No host tags found.\n".to_string();
        }
        let mut out = format!("{:<38} {:<30} {:<10} HOSTS\n", "ID", "NAME", "COLOR");
        for t in &self.items {
            out.push_str(&format!(
                "{:<38} {:<30} {:<10} {}\n",
                t.id, t.name, t.color, t.host_count
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for HostTagResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:          {}\n", self.id));
        out.push_str(&format!("Name:        {}\n", self.name));
        out.push_str(&format!("Color:       {}\n", self.color));
        if let Some(ref desc) = self.description {
            out.push_str(&format!("Description: {}\n", desc));
        }
        out.push_str(&format!("Host Count:  {}\n", self.host_count));
        out.push_str(&format!(
            "Created:     {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out.push_str(&format!(
            "Updated:     {}\n",
            self.updated_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.updated_at.to_string())
        ));
        out
    }
}

impl HumanOutput for Vec<HostTagSummary> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No tags assigned.\n".to_string();
        }
        let mut out = format!("{:<38} {:<30} COLOR\n", "ID", "NAME");
        for t in self {
            out.push_str(&format!("{:<38} {:<30} {}\n", t.id, t.name, t.color));
        }
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub search: Option<String>,
}

pub struct ShowParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct CreateParams<'a> {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct UpdateParams<'a> {
    pub id: &'a Uuid,
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: Option<String>,
    pub clear_description: bool,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct DeleteParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct SetParams<'a> {
    pub host_id: &'a Uuid,
    pub tag_ids: Vec<Uuid>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List all host tags (paginated).
pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<HostTagResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let query = ListHostTagsQuery {
        page: params.page,
        per_page: params.per_page,
        search: params.search,
    };
    client.list_host_tags(&query).await.context_to()
}

/// Show details for a single host tag.
pub async fn show(params: ShowParams<'_>) -> Result<HostTagResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.get_host_tag(params.id).await.context_to()
}

/// Create a new host tag.
pub async fn create(params: CreateParams<'_>) -> Result<HostTagResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateHostTagRequest {
        name: params.name,
        color: params.color,
        description: params.description,
    };
    client.create_host_tag(&req).await.context_to()
}

/// Update an existing host tag.
pub async fn update(params: UpdateParams<'_>) -> Result<HostTagResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let description = if params.clear_description {
        Some(serde_json::Value::Null)
    } else {
        params.description.map(serde_json::Value::String)
    };
    let req = UpdateHostTagRequest {
        name: params.name,
        color: params.color,
        description,
    };
    client.update_host_tag(params.id, &req).await.context_to()
}

/// Delete a host tag.
pub async fn delete(params: DeleteParams<'_>) -> Result<HostMessageResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.delete_host_tag(params.id).await.context_to()?;
    Ok(HostMessageResponse {
        message: "Host tag deleted.".to_string(),
    })
}

/// Set tags on a host (replace-all).
pub async fn set_tags(params: SetParams<'_>) -> Result<Vec<HostTagSummary>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = SetHostTagsRequest {
        tag_ids: params.tag_ids,
    };
    client
        .set_host_tags(params.host_id, &req)
        .await
        .context_to()
}

/// Perform a batch action on multiple host tags.
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
    client.batch_host_tags(&req).await.context_to()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_tag() -> HostTagResponse {
        HostTagResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            name: "production".to_string(),
            color: "#3B82F6".to_string(),
            description: Some("Production hosts".to_string()),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            host_count: 5,
        }
    }

    #[test]
    fn host_tag_detail_human_output_contains_key_fields() {
        let tag = sample_tag();
        let s = tag.to_human_string();
        assert!(s.contains("production"), "name missing");
        assert!(s.contains("#3B82F6"), "color missing");
        assert!(s.contains("Production hosts"), "description missing");
        assert!(s.contains("5"), "host_count missing");
    }

    #[test]
    fn host_tag_list_human_output_empty() {
        let resp: PaginatedResponse<HostTagResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No host tags found"));
    }

    #[test]
    fn host_tag_list_human_output_with_items() {
        let resp = PaginatedResponse {
            items: vec![sample_tag()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let s = resp.to_human_string();
        assert!(s.contains("production"));
        assert!(s.contains("Page 1 of 1"));
    }

    #[test]
    fn tag_summary_vec_human_output() {
        let tags = vec![HostTagSummary {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            name: "staging".to_string(),
            color: "#EF4444".to_string(),
        }];
        let s = tags.to_human_string();
        assert!(s.contains("staging"));
        assert!(s.contains("#EF4444"));
    }

    #[test]
    fn tag_summary_vec_empty_human_output() {
        let tags: Vec<HostTagSummary> = vec![];
        assert!(tags.to_human_string().contains("No tags assigned"));
    }
}
