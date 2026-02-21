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

#[cfg(test)]
mod tests {
    use super::*;

    // ── NetworkSettingsResponse ───────────────────────────────────────

    #[test]
    fn network_response_round_trip_all_fields() {
        let resp = NetworkSettingsResponse {
            trusted_proxies: vec!["10.0.0.0/8".to_string()],
            real_ip_header: "X-Forwarded-For".to_string(),
            extra_sans: vec!["example.com".to_string()],
            https_addr: "0.0.0.0:8443".to_string(),
            forwarded_client_cert_info_header: Some("X-Forwarded-Tls-Client-Cert-Info".to_string()),
            forwarded_client_cert_pem_header: Some("X-Forwarded-Tls-Client-Cert".to_string()),
            pki_addr: Some("http://pki.example.com".to_string()),
            pki_addr_warning: Some("CA rotation required".to_string()),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: NetworkSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.trusted_proxies, vec!["10.0.0.0/8"]);
        assert_eq!(de.real_ip_header, "X-Forwarded-For");
        assert_eq!(de.https_addr, "0.0.0.0:8443");
        assert!(de.pki_addr.is_some());
        assert!(de.pki_addr_warning.is_some());
    }

    #[test]
    fn network_response_round_trip_none_fields() {
        let resp = NetworkSettingsResponse {
            trusted_proxies: vec![],
            real_ip_header: "X-Real-IP".to_string(),
            extra_sans: vec![],
            https_addr: "0.0.0.0:443".to_string(),
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: None,
            pki_addr_warning: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: NetworkSettingsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(de.trusted_proxies.is_empty());
        assert!(de.forwarded_client_cert_info_header.is_none());
        assert!(de.pki_addr.is_none());
    }

    // ── UpdateNetworkSettingsRequest ──────────────────────────────────

    #[test]
    fn update_request_round_trip() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: Some(vec!["192.168.0.0/16".to_string()]),
            real_ip_header: Some("X-Real-IP".to_string()),
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some("https://pki.example.com".to_string()),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: UpdateNetworkSettingsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(
            de.trusted_proxies.as_deref(),
            Some(vec!["192.168.0.0/16".to_string()].as_slice())
        );
        assert_eq!(de.pki_addr.as_deref(), Some("https://pki.example.com"));
    }

    #[test]
    fn validate_rejects_empty_proxy_items() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: Some(vec!["10.0.0.0/8".to_string(), String::new()]),
            real_ip_header: None,
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: None,
        };
        let err = req.validate().expect_err("should reject empty proxy item");
        assert_eq!(err.field, "trusted_proxies");
    }

    #[test]
    fn validate_rejects_empty_real_ip_header() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: None,
            real_ip_header: Some(String::new()),
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: None,
        };
        let err = req
            .validate()
            .expect_err("should reject empty real_ip_header");
        assert_eq!(err.field, "real_ip_header");
    }

    #[test]
    fn validate_rejects_invalid_pki_addr_scheme() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: None,
            real_ip_header: None,
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some("ftp://bad.example.com".to_string()),
        };
        let err = req.validate().expect_err("should reject ftp:// scheme");
        assert_eq!(err.field, "pki_addr");
    }

    #[test]
    fn validate_accepts_http_pki_addr() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: None,
            real_ip_header: None,
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some("http://pki.local".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_https_pki_addr() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: None,
            real_ip_header: None,
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some("https://pki.example.com".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_empty_pki_addr_passthrough() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: None,
            real_ip_header: None,
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some(String::new()),
        };
        assert!(
            req.validate().is_ok(),
            "empty string should pass through (disables pki_addr)"
        );
    }

    #[test]
    fn validate_accepts_none_fields() {
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: None,
            real_ip_header: None,
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: None,
        };
        assert!(req.validate().is_ok());
    }
}
