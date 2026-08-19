//! Request/response types for the OAuth global-settings API.
//!
//! `GET /api/v1/global-settings/oauth` returns [`OAuthSettingsResponse`].
//! `PUT /api/v1/global-settings/oauth` accepts [`UpdateOAuthSettingsRequest`].
//!
//! These settings control the MCP OAuth 2.1 authorization-server feature.
//! All fields are stored in `global_settings` and take effect after the
//! controller is restarted.

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

/// Response body for `GET /api/v1/global-settings/oauth`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthSettingsResponse {
    /// Whether the MCP OAuth 2.1 authorization server is enabled.
    /// When `false` all `/oauth/*` and `/api/oauth/*` routes return 404.
    pub mcp_enabled: bool,
    /// Whether Dynamic Client Registration (`POST /oauth/register`) is open.
    pub dcr_enabled: bool,
    /// Whether Client Initiated Metadata Discovery is active.
    pub cimd_enabled: bool,
    /// Canonical hostname used as the `iss` / `aud` claim in OAuth tokens.
    /// `None` when not yet configured (required before `mcp_enabled` can be `true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_host: Option<String>,
    /// `true` when the persisted DB values differ from the values that were
    /// loaded at boot time — changes will not take full effect until restart.
    pub restart_required: bool,
}

/// Request body for `PUT /api/v1/global-settings/oauth`.
///
/// All fields are optional — omitted fields keep their current value.
///
/// - `canonical_host`: empty string clears the value; non-empty must be a
///   plain hostname or `host:port` (no scheme).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateOAuthSettingsRequest {
    /// Enable or disable the MCP OAuth AS. `None` = keep existing.
    pub mcp_enabled: Option<bool>,
    /// Enable or disable DCR. `None` = keep existing.
    pub dcr_enabled: Option<bool>,
    /// Enable or disable CIMD. `None` = keep existing.
    pub cimd_enabled: Option<bool>,
    /// Canonical hostname for token claims.
    /// Empty string clears; `None` = keep existing.
    pub canonical_host: Option<String>,
}

impl Validate for UpdateOAuthSettingsRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(host) = &self.canonical_host {
            let trimmed = host.trim();
            if !trimmed.is_empty() && !crate::oauth::canonical_url::is_bare_host(trimmed) {
                return Err(ValidationError {
                    field: "canonical_host",
                    message: "must be a bare host, optionally with a port".to_string(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    #[test]
    fn response_round_trip() {
        let resp = OAuthSettingsResponse {
            mcp_enabled: true,
            dcr_enabled: false,
            cimd_enabled: true,
            canonical_host: Some("auth.example.com".to_string()),
            restart_required: false,
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let de: OAuthSettingsResponse = serde_json::from_str(&json).expect("deserialize");
        assert!(de.mcp_enabled);
        assert!(!de.dcr_enabled);
        assert!(de.cimd_enabled);
        assert_eq!(de.canonical_host.as_deref(), Some("auth.example.com"));
        assert!(!de.restart_required);
    }

    #[test]
    fn response_none_canonical_host_omitted() {
        let resp = OAuthSettingsResponse {
            mcp_enabled: false,
            dcr_enabled: false,
            cimd_enabled: false,
            canonical_host: None,
            restart_required: false,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert!(
            json.get("canonical_host").is_none(),
            "absent field must not appear"
        );
    }

    #[test]
    fn validate_accepts_plain_hostname() {
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("auth.example.com".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_host_with_port() {
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("auth.example.com:8443".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_empty_string_to_clear() {
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some(String::new()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_rejects_url_with_scheme() {
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("https://auth.example.com".to_string()),
        };
        let err = req.validate().expect_err("should reject scheme");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_rejects_hostname_with_spaces() {
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("auth example.com".to_string()),
        };
        let err = req.validate().expect_err("should reject spaces");
        assert_eq!(err.field, "canonical_host");
    }

    // -- Canonical-host shape gate (is_bare_host, via validate) --

    #[test]
    fn validate_rejects_host_with_path() {
        // Isolates the `/` clause of is_bare_host.
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("example.com/app".to_string()),
        };
        let err = req.validate().expect_err("should reject path segment");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_rejects_host_with_backslash() {
        // Isolates the `\` clause of is_bare_host: the WHATWG URL parser
        // treats a backslash as a path separator for special schemes, so
        // without this clause the value would parse to host `example.com`
        // and silently drop `\app`.
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("example.com\\app".to_string()),
        };
        let err = req.validate().expect_err("should reject backslash path");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_rejects_host_with_userinfo() {
        // Isolates the `@` clause of is_bare_host.
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("user@example.com".to_string()),
        };
        let err = req.validate().expect_err("should reject userinfo");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_rejects_host_with_query() {
        // Isolates the `?` clause of is_bare_host.
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("example.com?x=1".to_string()),
        };
        let err = req.validate().expect_err("should reject query string");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_rejects_host_with_fragment() {
        // Isolates the `#` clause of is_bare_host.
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("example.com#f".to_string()),
        };
        let err = req.validate().expect_err("should reject fragment");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_rejects_host_with_embedded_space() {
        // Isolates the whitespace clause of is_bare_host (distinct from the
        // legacy `contains(' ')` check this replaces).
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("a b.example.com".to_string()),
        };
        let err = req.validate().expect_err("should reject embedded space");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_rejects_scheme() {
        // Contains `/` (from `//`), so this also exercises the `/` clause —
        // kept as its own fixture since it documents the "no scheme" intent.
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("http://example.com".to_string()),
        };
        let err = req.validate().expect_err("should reject scheme prefix");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_rejects_unparseable_host() {
        // Isolates the `url::Url::parse(...).is_ok_and(...)` clause: no
        // forbidden char, no whitespace, but an invalid (non-numeric) port
        // makes the resulting URL fail to parse.
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("example.com:notaport".to_string()),
        };
        let err = req.validate().expect_err("should reject unparseable host");
        assert_eq!(err.field, "canonical_host");
    }

    #[test]
    fn validate_accepts_bare_host() {
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("example.com".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_bare_host_with_port() {
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("example.com:8443".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_bracketed_ipv6_host_with_port() {
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("[::1]:8443".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_trailing_colon() {
        // Pinned deliberately: `url::Url::parse("https://example.com:")`
        // succeeds and re-parses to the same host everywhere downstream, so
        // the shape gate tolerates a trailing colon with no port digits.
        let req = UpdateOAuthSettingsRequest {
            mcp_enabled: None,
            dcr_enabled: None,
            cimd_enabled: None,
            canonical_host: Some("example.com:".to_string()),
        };
        assert!(req.validate().is_ok());
    }
}
