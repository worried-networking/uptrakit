use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::host_packages::{
    CreateHostPackageIgnoreRequest, HostPackageDetailResponse, HostPackageIgnoreResponse,
    HostPackageResponse, ListHostPackagesParams, UpdateHostPackageRequest,
};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uuid::Uuid;

/// Simple wrapper for status messages that implements `Serialize` + `HumanOutput`.
#[derive(Debug, serde::Serialize)]
pub struct StatusMessage {
    pub message: String,
}

impl HumanOutput for StatusMessage {
    fn to_human_string(&self) -> String {
        format!("{}\n", self.message)
    }
}

// ---------------------------------------------------------------------------
// HumanOutput implementations
// ---------------------------------------------------------------------------

impl HumanOutput for PaginatedResponse<HostPackageResponse> {
    fn to_human_string(&self) -> String {
        if self.items.is_empty() {
            return "No host packages found.\n".to_string();
        }
        let mut out = format!(
            "{:<38} {:<30} {:<15} {:<15} {:<10}\n",
            "ID", "NAME", "INSTALLED", "LATEST", "ENABLED"
        );
        for p in &self.items {
            let installed = p.installed_version.as_deref().unwrap_or("-");
            let latest = p.latest_version.as_deref().unwrap_or("-");
            let enabled = if p.enabled { "yes" } else { "no" };
            out.push_str(&format!(
                "{:<38} {:<30} {:<15} {:<15} {:<10}\n",
                p.id,
                truncate(&p.name, 28),
                truncate(installed, 13),
                truncate(latest, 13),
                enabled,
            ));
        }
        out.push_str(&format!(
            "\nPage {} of {} ({} total)\n",
            self.page, self.total_pages, self.total
        ));
        out
    }
}

impl HumanOutput for HostPackageDetailResponse {
    fn to_human_string(&self) -> String {
        let p = &self.package;
        let mut out = format!(
            "ID:                  {}\n\
             Name:                {}\n\
             Package Identifier:  {}\n\
             Installed Version:   {}\n\
             Latest Version:      {}\n\
             Update Category:     {}\n\
             Enabled:             {}\n",
            p.id,
            p.name,
            p.package_identifier,
            p.installed_version.as_deref().unwrap_or("-"),
            p.latest_version.as_deref().unwrap_or("-"),
            p.update_category,
            if p.enabled { "yes" } else { "no" },
        );
        if let Some(ref checked) = p.last_checked_at {
            out.push_str(&format!("Last Checked:        {checked}\n"));
        }
        if let Some(ref updated) = p.last_updated_at {
            out.push_str(&format!("Last Updated:        {updated}\n"));
        }
        if !self.recent_updates.is_empty() {
            out.push_str("\nRecent Updates:\n");
            for u in &self.recent_updates {
                let status = &u.status;
                let from = u.from_version.as_deref().unwrap_or("-");
                let to = u.to_version.as_deref().unwrap_or("-");
                out.push_str(&format!(
                    "  [{status}] {from} → {to}  ({0})\n",
                    u.created_at
                ));
            }
        }
        out
    }
}

impl HumanOutput for HostPackageResponse {
    fn to_human_string(&self) -> String {
        format!(
            "ID:                  {}\n\
             Name:                {}\n\
             Package Identifier:  {}\n\
             Installed Version:   {}\n\
             Latest Version:      {}\n\
             Update Category:     {}\n\
             Enabled:             {}\n",
            self.id,
            self.name,
            self.package_identifier,
            self.installed_version.as_deref().unwrap_or("-"),
            self.latest_version.as_deref().unwrap_or("-"),
            self.update_category,
            if self.enabled { "yes" } else { "no" },
        )
    }
}

impl HumanOutput for Vec<HostPackageIgnoreResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No ignore rules found.\n".to_string();
        }
        let mut out = format!("{:<38} {:<38} {:<30}\n", "ID", "PLUGIN CONFIG", "PACKAGE");
        for r in self {
            out.push_str(&format!(
                "{:<38} {:<38} {:<30}\n",
                r.id, r.plugin_config_id, r.package_identifier
            ));
        }
        out
    }
}

impl HumanOutput for HostPackageIgnoreResponse {
    fn to_human_string(&self) -> String {
        format!(
            "ID:                  {}\n\
             Plugin Config ID:    {}\n\
             Package Identifier:  {}\n\
             Created At:          {}\n",
            self.id, self.plugin_config_id, self.package_identifier, self.created_at,
        )
    }
}

fn truncate(s: &str, len: usize) -> String {
    if s.len() > len {
        format!("{}…", &s[..len.saturating_sub(1)])
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

pub struct ListParams<'a> {
    pub host_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub enabled: Option<bool>,
    pub has_update: Option<bool>,
    pub category: Option<String>,
    pub search: Option<String>,
}

pub struct ShowParams<'a> {
    pub host_id: &'a Uuid,
    pub package_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct UpdateParams<'a> {
    pub host_id: &'a Uuid,
    pub package_id: &'a Uuid,
    pub enabled: bool,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct DeleteParams<'a> {
    pub host_id: &'a Uuid,
    pub package_id: &'a Uuid,
    pub ignore: bool,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ListIgnoresParams<'a> {
    pub host_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct AddIgnoreParams<'a> {
    pub host_id: &'a Uuid,
    pub plugin_config_id: &'a Uuid,
    pub package_identifier: String,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct RemoveIgnoreParams<'a> {
    pub host_id: &'a Uuid,
    pub ignore_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ---------------------------------------------------------------------------
// Command functions
// ---------------------------------------------------------------------------

pub async fn list(params: ListParams<'_>) -> Result<PaginatedResponse<HostPackageResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let query = ListHostPackagesParams {
        page: params.page,
        per_page: params.per_page,
        enabled: params.enabled,
        has_update: params.has_update,
        category: params.category,
        search: params.search,
    };
    client
        .list_host_packages(params.host_id, &query)
        .await
        .context_to()
}

pub async fn show(params: ShowParams<'_>) -> Result<HostPackageDetailResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .get_host_package(params.host_id, params.package_id)
        .await
        .context_to()
}

pub async fn update(params: UpdateParams<'_>) -> Result<HostPackageResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateHostPackageRequest {
        enabled: params.enabled,
    };
    client
        .update_host_package(params.host_id, params.package_id, &req)
        .await
        .context_to()
}

pub async fn delete(params: DeleteParams<'_>) -> Result<StatusMessage> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .delete_host_package(params.host_id, params.package_id, params.ignore)
        .await
        .context_to()?;
    let message = if params.ignore {
        "Host package deleted and ignore rule created."
    } else {
        "Host package deleted."
    };
    Ok(StatusMessage {
        message: message.to_string(),
    })
}

pub async fn list_ignores(params: ListIgnoresParams<'_>) -> Result<Vec<HostPackageIgnoreResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .list_host_package_ignores(params.host_id)
        .await
        .context_to()
}

pub async fn add_ignore(params: AddIgnoreParams<'_>) -> Result<HostPackageIgnoreResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateHostPackageIgnoreRequest {
        plugin_config_id: *params.plugin_config_id,
        package_identifier: params.package_identifier,
    };
    client
        .create_host_package_ignore(params.host_id, &req)
        .await
        .context_to()
}

pub async fn remove_ignore(params: RemoveIgnoreParams<'_>) -> Result<StatusMessage> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .delete_host_package_ignore(params.host_id, params.ignore_id)
        .await
        .context_to()?;
    Ok(StatusMessage {
        message: "Ignore rule removed.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_openapi_client::types::pagination::PaginatedResponse;

    fn sample_package() -> HostPackageResponse {
        HostPackageResponse {
            id: Uuid::nil(),
            host_id: Uuid::nil(),
            plugin_config_id: Uuid::nil(),
            package_identifier: "nginx".to_string(),
            name: "nginx".to_string(),
            installed_version: Some("1.22.0".to_string()),
            installed_version_detected_at: None,
            latest_version: Some("1.24.0".to_string()),
            latest_version_fetched_at: None,
            update_category: "standard".to_string(),
            enabled: true,
            last_checked_at: None,
            last_updated_at: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            has_update: true,
        }
    }

    #[test]
    fn paginated_list_human_output() {
        let resp: PaginatedResponse<HostPackageResponse> = PaginatedResponse {
            items: vec![sample_package()],
            total: 1,
            page: 1,
            per_page: 20,
            total_pages: 1,
        };
        let output = resp.to_human_string();
        assert!(output.contains("nginx"));
        assert!(output.contains("1.22.0"));
        assert!(output.contains("1.24.0"));
        assert!(output.contains("Page 1 of 1"));
    }

    #[test]
    fn empty_list_human_output() {
        let resp: PaginatedResponse<HostPackageResponse> = PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: 20,
            total_pages: 0,
        };
        let output = resp.to_human_string();
        assert!(output.contains("No host packages found"));
    }

    #[test]
    fn single_package_human_output() {
        let pkg = sample_package();
        let output = pkg.to_human_string();
        assert!(output.contains("nginx"));
        assert!(output.contains("1.22.0"));
        assert!(output.contains("yes"));
    }

    #[test]
    fn ignore_list_empty() {
        let ignores: Vec<HostPackageIgnoreResponse> = vec![];
        let output = ignores.to_human_string();
        assert!(output.contains("No ignore rules found"));
    }
}
