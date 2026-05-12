//! RFC 8628 (Device Authorization Grant) + RFC 8414 (Authorization Server
//! Metadata) request/response types.
//!
//! See `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_macros::wire_safe_enum;

use crate::validation::{Validate, ValidationError};

// --- Error codes --------------------------------------------------------

wire_safe_enum! {
    /// OAuth 2.0 error codes per RFC 6749 §5.2 and RFC 8628 §3.5.
    ///
    /// Wire-safe via `Other(String)` so the CLI tolerates new codes added
    /// by a newer server.
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
    pub enum OAuthErrorCode {
        AuthorizationPending => "authorization_pending",
        SlowDown             => "slow_down",
        AccessDenied         => "access_denied",
        ExpiredToken         => "expired_token",
        InvalidRequest       => "invalid_request",
        InvalidClient        => "invalid_client",
        InvalidGrant         => "invalid_grant",
        UnsupportedGrantType => "unsupported_grant_type",
        ServerError          => "server_error",
    }
    parse_error = ParseOAuthErrorCodeError("invalid OAuth 2.0 error code");
}

// --- Device-authorization request / response ---------------------------

/// RFC 8628 §3.1 device-authorization request. Form-urlencoded body.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthorizationRequest {
    /// Public client identifier. Must match the server's configured constant.
    pub client_id: String,
    /// Optional space-separated scope list (RFC 6749 §3.3). Stored on the flow
    /// row, echoed on the token response, not yet enforced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Uptrakit extension: free-form audit label, e.g. `cli-laptop-2026-05-12`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
}

impl DeviceAuthorizationRequest {
    /// Construct a `DeviceAuthorizationRequest`.
    pub fn new(client_id: String, scope: Option<String>, client_name: Option<String>) -> Self {
        Self {
            client_id,
            scope,
            client_name,
        }
    }
}

impl Validate for DeviceAuthorizationRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.client_id.trim().is_empty() {
            return Err(ValidationError {
                field: "client_id",
                message: "client_id is required".to_string(),
            });
        }
        if self.scope.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return Err(ValidationError {
                field: "scope",
                message: "scope must be non-empty when present".to_string(),
            });
        }
        Ok(())
    }
}

/// RFC 8628 §3.2 device-authorization response.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: i32,
}

impl DeviceAuthorizationResponse {
    /// Construct a `DeviceAuthorizationResponse`.
    pub fn new(
        device_code: String,
        user_code: String,
        verification_uri: String,
        verification_uri_complete: String,
        expires_in: u64,
        interval: i32,
    ) -> Self {
        Self {
            device_code,
            user_code,
            verification_uri,
            verification_uri_complete,
            expires_in,
            interval,
        }
    }
}

// --- Token request / response ------------------------------------------

/// RFC 6749 §3.2 / RFC 8628 §3.4 token endpoint request. Form-urlencoded.
///
/// `grant_type` is intentionally `String` — the device-code grant value is the
/// literal URI `urn:ietf:params:oauth:grant-type:device_code`; the handler
/// matches the raw string and returns `unsupported_grant_type` for any other
/// value.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthTokenRequest {
    pub grant_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

impl OAuthTokenRequest {
    /// Construct an `OAuthTokenRequest`.
    pub fn new(grant_type: String, device_code: Option<String>, client_id: Option<String>) -> Self {
        Self {
            grant_type,
            device_code,
            client_id,
        }
    }
}

impl Validate for OAuthTokenRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.grant_type.trim().is_empty() {
            return Err(ValidationError {
                field: "grant_type",
                message: "grant_type is required".to_string(),
            });
        }
        Ok(())
    }
}

/// RFC 6749 §5.1 success token response.
///
/// `expires_in`, `refresh_token`, and `scope` are `Option` + `skip_serializing_if`
/// so they are omitted (not serialised as `null`) when unset. Today the server
/// always omits all three; the fields exist on the wire type so a future
/// migration to short-lived bearer + refresh tokens is purely additive (Seam 1).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl OAuthTokenResponse {
    /// Construct an `OAuthTokenResponse` with the required fields.
    ///
    /// `expires_in`, `refresh_token`, and `scope` default to `None`.
    pub fn new(access_token: String, token_type: String) -> Self {
        Self {
            access_token,
            token_type,
            expires_in: None,
            refresh_token: None,
            scope: None,
        }
    }
}

/// RFC 6749 §5.2 error response, with the uptrakit `interval` extension used
/// by `slow_down` (RFC 8628 §3.5).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthErrorResponse {
    pub error: OAuthErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
    /// Server-recommended polling interval (seconds). Only set on `slow_down`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<i32>,
}

impl OAuthErrorResponse {
    /// Construct an `OAuthErrorResponse`.
    pub fn new(
        error: OAuthErrorCode,
        error_description: Option<String>,
        interval: Option<i32>,
    ) -> Self {
        Self {
            error,
            error_description,
            interval,
        }
    }
}

// --- Discovery metadata ------------------------------------------------

/// RFC 8414 §3 authorization server metadata (device-grant-only subset).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthAuthorizationServerMetadata {
    pub issuer: String,
    pub device_authorization_endpoint: String,
    pub token_endpoint: String,
    pub grant_types_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
}

impl OAuthAuthorizationServerMetadata {
    /// Construct an `OAuthAuthorizationServerMetadata`.
    pub fn new(
        issuer: String,
        device_authorization_endpoint: String,
        token_endpoint: String,
        grant_types_supported: Vec<String>,
        response_types_supported: Vec<String>,
        token_endpoint_auth_methods_supported: Vec<String>,
        code_challenge_methods_supported: Vec<String>,
    ) -> Self {
        Self {
            issuer,
            device_authorization_endpoint,
            token_endpoint,
            grant_types_supported,
            response_types_supported,
            token_endpoint_auth_methods_supported,
            code_challenge_methods_supported,
        }
    }
}

// --- UI-internal: deny + lookup ---------------------------------------

/// Consonant alphabet used for user-code generation and validation.
/// Shared with the device-flow store so both sides enforce the same charset.
pub const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

/// Validate that a `user_code` string matches the `XXXX-XXXX` format:
/// exactly 9 characters, a dash at position 4, consonants (no vowels) elsewhere.
fn validate_user_code_format(user_code: &str) -> Result<(), ValidationError> {
    let bytes = user_code.as_bytes();
    let valid = bytes.len() == 9
        && bytes.get(4).copied() == Some(b'-')
        && bytes
            .get(..4)
            .is_some_and(|s| s.iter().all(|b| USER_CODE_ALPHABET.contains(b)))
        && bytes
            .get(5..)
            .is_some_and(|s| s.iter().all(|b| USER_CODE_ALPHABET.contains(b)));
    if !valid {
        return Err(ValidationError {
            field: "user_code",
            message: "user_code must be in XXXX-XXXX format (uppercase consonants)".to_string(),
        });
    }
    Ok(())
}

/// Request body for `POST /api/v1/auth/device/deny` (UI-internal).
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthDenyRequest {
    pub user_code: String,
}

impl DeviceAuthDenyRequest {
    /// Construct a `DeviceAuthDenyRequest`.
    pub fn new(user_code: String) -> Self {
        Self { user_code }
    }
}

impl Validate for DeviceAuthDenyRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_user_code_format(&self.user_code)
    }
}

/// Response body for `POST /api/v1/auth/device/deny`.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthDenyResponse {
    pub message: String,
}

impl DeviceAuthDenyResponse {
    /// Construct a `DeviceAuthDenyResponse`.
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

/// Query string for `GET /api/v1/auth/device/lookup` (UI-internal).
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct DeviceAuthLookupQuery {
    pub user_code: String,
}

impl Validate for DeviceAuthLookupQuery {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_user_code_format(&self.user_code)
    }
}

/// Response body for `GET /api/v1/auth/device/lookup`.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceAuthLookupResponse {
    pub client_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub expires_at: OffsetDateTime,
}

impl DeviceAuthLookupResponse {
    /// Construct a `DeviceAuthLookupResponse`.
    pub fn new(client_name: Option<String>, expires_at: OffsetDateTime) -> Self {
        Self {
            client_name,
            expires_at,
        }
    }
}

// --- Tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN_VARIANTS: &[&str] = &[
        "authorization_pending",
        "slow_down",
        "access_denied",
        "expired_token",
        "invalid_request",
        "invalid_client",
        "invalid_grant",
        "unsupported_grant_type",
        "server_error",
    ];

    #[test]
    fn oauth_error_code_known_variants_round_trip() {
        for wire in KNOWN_VARIANTS {
            let value = OAuthErrorCode::from((*wire).to_string());
            assert_eq!(value.as_str(), *wire);
            let json = serde_json::to_string(&value).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""));
            let parsed: OAuthErrorCode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, value);
        }
    }

    #[test]
    fn oauth_error_code_unknown_deserializes_to_other() {
        let json = "\"temporarily_unavailable\"";
        let value: OAuthErrorCode = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            value,
            OAuthErrorCode::Other("temporarily_unavailable".into())
        );
        // Round-trip preserves the inner string.
        assert_eq!(serde_json::to_string(&value).expect("serialize"), json);
    }

    #[test]
    fn validate_rejects_empty_client_id() {
        let req = DeviceAuthorizationRequest {
            client_id: "".into(),
            scope: None,
            client_name: None,
        };
        req.validate().unwrap_err();
    }

    #[test]
    fn validate_rejects_empty_grant_type() {
        let req = OAuthTokenRequest {
            grant_type: "".into(),
            device_code: None,
            client_id: None,
        };
        req.validate().unwrap_err();
    }

    #[test]
    fn token_response_omits_optional_fields() {
        let resp = OAuthTokenResponse {
            access_token: "abc".into(),
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
            scope: None,
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["access_token"], "abc");
        assert_eq!(json["token_type"], "Bearer");
        assert!(
            json.get("expires_in").is_none(),
            "expires_in must be omitted"
        );
        assert!(
            json.get("refresh_token").is_none(),
            "refresh_token must be omitted"
        );
        assert!(json.get("scope").is_none(), "scope must be omitted");
    }

    #[test]
    fn validate_user_code_accepts_consonant_code() {
        let req = DeviceAuthDenyRequest {
            user_code: "BCDF-GHJK".into(),
        };
        req.validate().expect("valid consonant code should pass");
    }

    #[test]
    fn validate_user_code_rejects_vowels() {
        for bad in &["AEIA-UUUU", "BCDA-GHJK", "BCDF-GHAK"] {
            let req = DeviceAuthDenyRequest {
                user_code: (*bad).into(),
            };
            req.validate()
                .expect_err(&format!("{bad} contains vowels — must fail"));
        }
    }

    #[test]
    fn validate_user_code_rejects_bad_format() {
        for bad in &["BCDFGHJK", "BCDF_GHJK", "bcdf-ghjk", "BCDF-GHJKL"] {
            let req = DeviceAuthDenyRequest {
                user_code: (*bad).into(),
            };
            req.validate()
                .expect_err(&format!("{bad} should fail format check"));
        }
    }

    #[test]
    fn error_response_with_slow_down_interval() {
        let resp = OAuthErrorResponse {
            error: OAuthErrorCode::SlowDown,
            error_description: None,
            interval: Some(10),
        };
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["error"], "slow_down");
        assert_eq!(json["interval"], 10);
        assert!(json.get("error_description").is_none());
    }
}
