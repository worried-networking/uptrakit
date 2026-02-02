use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NetworkSettingsResponse {
    pub trusted_proxies: Vec<String>,
    pub real_ip_header: String,
    pub extra_sans: Vec<String>,
    pub https_addr: String,
    pub forwarded_client_cert_info_header: Option<String>,
    pub forwarded_client_cert_pem_header: Option<String>,
    pub pki_addr: Option<String>,
    /// Warning message when pki_addr was changed, explaining that CA rotation is required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pki_addr_warning: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateNetworkSettingsRequest {
    pub trusted_proxies: Option<Vec<String>>,
    pub real_ip_header: Option<String>,
    pub extra_sans: Option<Vec<String>>,
    pub https_addr: Option<String>,
    /// Header name for structured client certificate info (e.g. `X-Forwarded-Tls-Client-Cert-Info`).
    /// Empty string disables.
    pub forwarded_client_cert_info_header: Option<String>,
    /// Header name for PEM-encoded client certificate (e.g. `X-Forwarded-Tls-Client-Cert`).
    /// Empty string disables.
    pub forwarded_client_cert_pem_header: Option<String>,
    /// URL for PKI endpoints (OCSP, CRL, CA cert) embedded in certificate extensions.
    /// Supports both http:// and https:// schemes.
    /// Empty string disables.
    pub pki_addr: Option<String>,
}
