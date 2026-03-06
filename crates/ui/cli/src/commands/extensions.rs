use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use rootcause::prelude::*;
use serde::Serialize;
use uptrakit_openapi_client::types::extensions::{ExtensionProviderInfo, ExtensionResponse};
use uuid::Uuid;

// ── HumanOutput impls ──────────────────────────────────────────────────────

impl HumanOutput for Vec<ExtensionResponse> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No extensions registered.\n".to_string();
        }
        let mut out = String::new();
        out.push_str(&format!(
            "{:<40} {:<30} {:<15} {:<10}\n",
            "ID", "Label", "Placement", "Providers"
        ));
        out.push_str(&format!("{}\n", "-".repeat(95)));
        for ext in self {
            let placement = format!("{:?}", ext.manifest.placement);
            // Truncate placement type for display.
            let placement = placement
                .split_once(['{', '('])
                .map_or(placement.as_str(), |(prefix, _)| prefix.trim());
            out.push_str(&format!(
                "{:<40} {:<30} {:<15} {:<10}\n",
                ext.manifest.id, ext.manifest.label, placement, ext.provider_count
            ));
        }
        out
    }
}

impl HumanOutput for Vec<ExtensionProviderInfo> {
    fn to_human_string(&self) -> String {
        if self.is_empty() {
            return "No providers connected.\n".to_string();
        }
        let mut out = String::new();
        out.push_str(&format!(
            "{:<38} {:<30} {}\n",
            "Service ID", "Label", "Hostname"
        ));
        out.push_str(&format!("{}\n", "-".repeat(80)));
        for p in self {
            out.push_str(&format!(
                "{:<38} {:<30} {}\n",
                p.service_id,
                p.service_label,
                p.hostname.as_deref().unwrap_or("-")
            ));
        }
        out
    }
}

/// Wrapper for invoke action response output.
#[derive(Debug, Serialize)]
pub struct InvokeOutput(pub serde_json::Value);

impl HumanOutput for InvokeOutput {
    fn to_human_string(&self) -> String {
        serde_json::to_string_pretty(&self.0).unwrap_or_else(|_| self.0.to_string()) + "\n"
    }
}

// ── Params ─────────────────────────────────────────────────────────────────

pub struct ListParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct ProvidersParams<'a> {
    pub extension_id: String,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

pub struct InvokeParams<'a> {
    pub extension_id: String,
    pub action_id: String,
    pub params: serde_json::Value,
    pub service_id: Option<Uuid>,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
}

// ── Commands ───────────────────────────────────────────────────────────────

pub async fn list(params: ListParams<'_>) -> Result<Vec<ExtensionResponse>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client.list_extensions().await.context_to()
}

pub async fn providers(params: ProvidersParams<'_>) -> Result<Vec<ExtensionProviderInfo>> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    client
        .list_extension_providers(&params.extension_id)
        .await
        .context_to()
}

pub async fn invoke(params: InvokeParams<'_>) -> Result<InvokeOutput> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let result = client
        .invoke_extension_action(
            &params.extension_id,
            &params.action_id,
            &params.params,
            params.service_id.as_ref(),
        )
        .await
        .context_to()?;
    Ok(InvokeOutput(result))
}
