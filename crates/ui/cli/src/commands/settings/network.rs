use crate::client::authenticated_client;
use crate::error::Result;
use crate::output::HumanOutput;
use clap::Subcommand;
use rootcause::prelude::*;
use uptrakit_openapi_client::types::settings_network::{
    NetworkSettingsResponse, UpdateNetworkSettingsRequest,
};

#[derive(Debug, Subcommand)]
pub enum NetworkCommands {
    /// Show network settings
    Show,
    /// Update network settings
    Update {
        /// Comma-separated trusted proxy CIDRs
        #[arg(long)]
        trusted_proxies: Option<String>,
        /// Header name for extracting real client IP
        #[arg(long)]
        real_ip_header: Option<String>,
        /// Comma-separated Subject Alternative Names for the server certificate
        #[arg(long)]
        sans: Option<String>,
        /// HTTPS listen address
        #[arg(long)]
        https_addr: Option<String>,
        /// Header for forwarded client cert info
        #[arg(long)]
        fwd_cert_info_header: Option<String>,
        /// Header for forwarded client cert PEM
        #[arg(long)]
        fwd_cert_pem_header: Option<String>,
        /// PKI address for OCSP/CRL/CA cert
        #[arg(long)]
        pki_addr: Option<String>,
    },
}

// ── Params ───────────────────────────────────────────────────────────────────

/// Parameters for updating network settings.
pub struct NetworkUpdateParams<'a> {
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
    pub request_timeout: Option<std::time::Duration>,
    pub trusted_proxies: Option<Vec<String>>,
    pub real_ip_header: Option<String>,
    pub sans: Option<Vec<String>>,
    pub https_addr: Option<String>,
    pub fwd_cert_info_header: Option<String>,
    pub fwd_cert_pem_header: Option<String>,
    pub pki_addr: Option<String>,
}

// ── Human output ─────────────────────────────────────────────────────────────

impl HumanOutput for NetworkSettingsResponse {
    fn to_human_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Trusted Proxies:     {}\n",
            self.trusted_proxies.join(", ")
        ));
        out.push_str(&format!("Real IP Header:      {}\n", self.real_ip_header));
        out.push_str(&format!("SANs:                {}\n", self.sans.join(", ")));
        out.push_str(&format!("HTTPS Address:       {}\n", self.https_addr));
        out.push_str(&format!(
            "Fwd Cert Info Header: {}\n",
            self.forwarded_client_cert_info_header
                .as_deref()
                .unwrap_or("-")
        ));
        out.push_str(&format!(
            "Fwd Cert PEM Header: {}\n",
            self.forwarded_client_cert_pem_header
                .as_deref()
                .unwrap_or("-")
        ));
        out.push_str(&format!(
            "PKI Address:         {}\n",
            self.pki_addr.as_deref().unwrap_or("-")
        ));
        if let Some(ref warning) = self.pki_addr_warning {
            out.push_str(&format!("Warning:             {warning}\n"));
        }
        if self.cert_regenerated == Some(true) {
            out.push_str("Cert Regenerated:    yes\n");
        }
        out
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Show network settings.
pub async fn network_show(
    server: Option<&str>,
    token: Option<&str>,
    insecure: bool,
    request_timeout: Option<std::time::Duration>,
) -> Result<NetworkSettingsResponse> {
    let client = authenticated_client(server, token, insecure, request_timeout)?;
    client.get_network_settings().await.context_to()
}

/// Update network settings.
pub async fn network_update(params: NetworkUpdateParams<'_>) -> Result<NetworkSettingsResponse> {
    let client = authenticated_client(
        params.server,
        params.token,
        params.insecure,
        params.request_timeout,
    )?;
    let req = UpdateNetworkSettingsRequest {
        trusted_proxies: params.trusted_proxies,
        real_ip_header: params.real_ip_header,
        sans: params.sans,
        https_addr: params.https_addr,
        forwarded_client_cert_info_header: params.fwd_cert_info_header,
        forwarded_client_cert_pem_header: params.fwd_cert_pem_header,
        pki_addr: params.pki_addr,
        regenerate_cert: None,
    };
    client.update_network_settings(&req).await.context_to()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_settings_human_output() {
        let resp = NetworkSettingsResponse {
            trusted_proxies: vec!["10.0.0.0/8".to_string()],
            real_ip_header: "X-Forwarded-For".to_string(),
            sans: vec![],
            https_addr: "0.0.0.0:443".to_string(),
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some("https://pki.example.com".to_string()),
            pki_addr_warning: Some("CA rotation required".to_string()),
            cert_regenerated: None,
        };
        let s = resp.to_human_string();
        assert!(s.contains("10.0.0.0/8"), "trusted proxies missing");
        assert!(s.contains("pki.example.com"), "pki_addr missing");
        assert!(s.contains("CA rotation required"), "warning missing");
    }
}
