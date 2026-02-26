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
use uptrakit_openapi_client::types::pagination::PaginatedResponse;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for PaginatedResponse<AutodiscoveryIgnoreResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No autodiscovery ignore rules found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<25} {:<20} PACKAGE\n",
            "ID", "PLUGIN CONFIG", "TYPE"
        );
        for r in &self.items {
            out.push_str(&format!(
                "{:<38} {:<25} {:<20} {}\n",
                r.id, r.plugin_config_name, r.provider_type, r.package_identifier
            ));
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
        out.push_str(&format!(
            "Plugin Config:    {} ({})\n",
            self.plugin_config_name, self.plugin_config_id
        ));
        out.push_str(&format!("Provider Type:    {}\n", self.provider_type));
        out.push_str(&format!("Package:          {}\n", self.package_identifier));
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
    pub provider_config_id: Option<&'a Uuid>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

pub struct IgnoresCreateParams<'a> {
    pub provider_config_id: &'a Uuid,
    pub package_identifier: String,
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
        plugin_config_id: params.provider_config_id,
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
    let req = CreateAutodiscoveryIgnoreRequest {
        plugin_config_id: *params.provider_config_id,
        package_identifier: params.package_identifier,
    };
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
            plugin_config_id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                .parse::<Uuid>()
                .unwrap(),
            plugin_config_name: "My GitHub".to_string(),
            provider_type: "github_releases".to_string(),
            package_identifier: "org/my-app".to_string(),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    #[test]
    fn ignore_detail_human_output() {
        let r = sample_ignore();
        let s = r.to_human_string();
        assert!(s.contains("My GitHub"), "plugin config name missing");
        assert!(s.contains("org/my-app"), "package identifier missing");
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
        assert!(s.contains("org/my-app"), "package identifier missing");
    }
}
