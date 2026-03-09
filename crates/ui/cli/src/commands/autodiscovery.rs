use crate::client::authenticated_client;
use crate::commands::settings::DeletedOutput;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::autodiscovery::ListIgnoresParams;
use uptrakit_openapi_client::types::autodiscovery::{
    AutodiscoveryIgnoreResponse, CreateAutodiscoveryIgnoreRequest,
};
use uptrakit_openapi_client::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<AutodiscoveryIgnoreResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No autodiscovery ignore rules found.\n".to_string();
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

impl HumanOutput for AutodiscoveryIgnoreResponse {
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
) -> Result<PaginatedResponse<AutodiscoveryIgnoreResponse>> {
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
        .list_autodiscovery_ignores(&list_params)
        .await
        .context_to()
}

pub async fn ignores_create(
    params: IgnoresCreateParams<'_>,
) -> Result<AutodiscoveryIgnoreResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateAutodiscoveryIgnoreRequest { name: params.name };
    client.create_autodiscovery_ignore(&req).await.context_to()
}

pub async fn ignores_delete(params: IgnoresDeleteParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .delete_autodiscovery_ignore(params.id)
        .await
        .context_to()?;
    Ok(DeletedOutput {
        message: format!("Autodiscovery ignore rule {} deleted.", params.id),
    })
}

/// Perform a batch action on multiple autodiscovery ignore rules.
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
    client.batch_autodiscovery_ignores(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_ignore() -> AutodiscoveryIgnoreResponse {
        AutodiscoveryIgnoreResponse {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            name: "FreshRSS".to_string(),
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
        let resp: PaginatedResponse<AutodiscoveryIgnoreResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        assert!(resp.to_human_string().contains("No autodiscovery ignore"));
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
