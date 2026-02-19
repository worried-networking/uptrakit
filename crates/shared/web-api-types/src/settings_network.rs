use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

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

impl Validate for UpdateNetworkSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref proxies) = self.trusted_proxies {
            for item in proxies {
                if item.is_empty() {
                    return Err(ValidationError {
                        field: "trusted_proxies",
                        message: "items must not be empty".to_string(),
                    });
                }
            }
        }

        if let Some(ref header) = self.real_ip_header
            && header.is_empty()
        {
            return Err(ValidationError {
                field: "real_ip_header",
                message: "must not be empty".to_string(),
            });
        }

        if let Some(ref addr) = self.pki_addr
            && !addr.is_empty()
            && !addr.starts_with("http://")
            && !addr.starts_with("https://")
        {
            return Err(ValidationError {
                field: "pki_addr",
                message: "must start with http:// or https://".to_string(),
            });
        }

        Ok(())
    }
}
