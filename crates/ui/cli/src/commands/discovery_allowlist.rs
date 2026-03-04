use crate::client::authenticated_client;
use crate::commands::settings::DeletedOutput;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use time::format_description::well_known::Rfc3339;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::types::discovery_allowlist::{
    CreateDiscoveryAllowlistEntryRequest, HostDiscoveryAllowlistEntry,
    TenantDiscoveryAllowlistEntry,
};
use uptrakit_shared_types::PluginType;

// ── Human output ────────────────────────────────────────────────────────────

impl HumanOutput for Vec<TenantDiscoveryAllowlistEntry> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No restrictions — all discovery plugins are active.\n".to_string();
        }
        let mut out = format!("{:<38} PLUGIN TYPE\n", "ID");
        for e in self {
            out.push_str(&format!("{:<38} {}\n", e.id, e.plugin_type));
        }
        out
    }
}

impl HumanOutput for TenantDiscoveryAllowlistEntry {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:          {}\n", self.id));
        out.push_str(&format!("Plugin Type: {}\n", self.plugin_type));
        out.push_str(&format!(
            "Created:     {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out
    }
}

impl HumanOutput for Vec<HostDiscoveryAllowlistEntry> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No host-specific restrictions — inherits tenant allowlist.\n".to_string();
        }
        let mut out = format!("{:<38} PLUGIN TYPE\n", "ID");
        for e in self {
            out.push_str(&format!("{:<38} {}\n", e.id, e.plugin_type));
        }
        out
    }
}

impl HumanOutput for HostDiscoveryAllowlistEntry {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("ID:          {}\n", self.id));
        out.push_str(&format!("Host:        {}\n", self.host_id));
        out.push_str(&format!("Plugin Type: {}\n", self.plugin_type));
        out.push_str(&format!(
            "Created:     {}\n",
            self.created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| self.created_at.to_string())
        ));
        out
    }
}

// ── Params ───────────────────────────────────────────────────────────────────

pub struct ListTenantParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct AddTenantParams<'a> {
    pub plugin_type: PluginType,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct RemoveTenantParams<'a> {
    pub id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ListHostParams<'a> {
    pub host_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct AddHostParams<'a> {
    pub host_id: &'a Uuid,
    pub plugin_type: PluginType,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct RemoveHostParams<'a> {
    pub host_id: &'a Uuid,
    pub entry_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// List tenant-wide discovery allowlist entries.
pub async fn tenant_list(
    params: ListTenantParams<'_>,
) -> Result<Vec<TenantDiscoveryAllowlistEntry>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.list_discovery_allowlist().await.context_to()
}

/// Add a plugin type to the tenant-wide discovery allowlist.
pub async fn tenant_add(params: AddTenantParams<'_>) -> Result<TenantDiscoveryAllowlistEntry> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateDiscoveryAllowlistEntryRequest {
        plugin_type: params.plugin_type,
    };
    client
        .add_discovery_allowlist_entry(&req)
        .await
        .context_to()
}

/// Remove a tenant-wide discovery allowlist entry.
pub async fn tenant_remove(params: RemoveTenantParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .remove_discovery_allowlist_entry(params.id)
        .await
        .context_to()?;
    Ok(DeletedOutput {
        message: format!("Discovery allowlist entry {} removed.", params.id),
    })
}

/// List host-specific discovery allowlist entries.
pub async fn host_list(params: ListHostParams<'_>) -> Result<Vec<HostDiscoveryAllowlistEntry>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .list_host_discovery_allowlist(params.host_id)
        .await
        .context_to()
}

/// Add a plugin type to a host's discovery allowlist.
pub async fn host_add(params: AddHostParams<'_>) -> Result<HostDiscoveryAllowlistEntry> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = CreateDiscoveryAllowlistEntryRequest {
        plugin_type: params.plugin_type,
    };
    client
        .add_host_discovery_allowlist_entry(params.host_id, &req)
        .await
        .context_to()
}

/// Remove a host-specific discovery allowlist entry.
pub async fn host_remove(params: RemoveHostParams<'_>) -> Result<DeletedOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .remove_host_discovery_allowlist_entry(params.host_id, params.entry_id)
        .await
        .context_to()?;
    Ok(DeletedOutput {
        message: format!(
            "Host discovery allowlist entry {} removed.",
            params.entry_id
        ),
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_tenant_entry() -> TenantDiscoveryAllowlistEntry {
        TenantDiscoveryAllowlistEntry {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            plugin_type: "package_manager_homebrew".to_string(),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    fn sample_host_entry() -> HostDiscoveryAllowlistEntry {
        HostDiscoveryAllowlistEntry {
            id: "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
                .parse::<Uuid>()
                .unwrap(),
            host_id: "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6"
                .parse::<Uuid>()
                .unwrap(),
            plugin_type: "package_manager_apt".to_string(),
            created_at: datetime!(2025-01-01 00:00:00 UTC),
        }
    }

    #[test]
    fn tenant_entry_human_output() {
        let e = sample_tenant_entry();
        let s = e.to_human_string();
        assert!(s.contains("package_manager_homebrew"));
        assert!(s.contains("Plugin Type:"));
    }

    #[test]
    fn tenant_list_empty_human_output() {
        let entries: Vec<TenantDiscoveryAllowlistEntry> = vec![];
        let s = entries.to_human_string();
        assert!(s.contains("No restrictions"));
    }

    #[test]
    fn tenant_list_non_empty_human_output() {
        let entries = vec![sample_tenant_entry()];
        let s = entries.to_human_string();
        assert!(s.contains("package_manager_homebrew"));
        assert!(s.contains("PLUGIN TYPE"));
    }

    #[test]
    fn host_entry_human_output() {
        let e = sample_host_entry();
        let s = e.to_human_string();
        assert!(s.contains("package_manager_apt"));
        assert!(s.contains("Host:"));
    }

    #[test]
    fn host_list_empty_human_output() {
        let entries: Vec<HostDiscoveryAllowlistEntry> = vec![];
        let s = entries.to_human_string();
        assert!(s.contains("inherits tenant allowlist"));
    }

    #[test]
    fn host_list_non_empty_human_output() {
        let entries = vec![sample_host_entry()];
        let s = entries.to_human_string();
        assert!(s.contains("package_manager_apt"));
    }
}
