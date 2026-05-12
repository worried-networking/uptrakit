//! HTTP response types for OAuth AS endpoints plus the Dynamic Client
//! Registration (RFC 7591) request envelope.
//!
//! `TokenResponse` is the JSON body returned by `POST /oauth/token`.
//! `DcrRegistrationRequest`/`Response` are the RFC 7591 register endpoint
//! envelopes; the request is `Validate`d per the project rule.

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

/// Successful token endpoint response (RFC 6749 §5.1).
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_expires_in: Option<i64>,
    pub scope: String,
}

/// Dynamic client registration request body (RFC 7591 §2).
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DcrRegistrationRequest {
    pub client_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_uri: Option<String>,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

const ALLOWED_GRANT_TYPES: &[&str] = &["authorization_code", "refresh_token"];
const ALLOWED_RESPONSE_TYPES: &[&str] = &["code"];
const ALLOWED_TOKEN_ENDPOINT_AUTH_METHODS: &[&str] = &["none", "client_secret_basic"];

impl Validate for DcrRegistrationRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.redirect_uris.is_empty() {
            return Err(ValidationError {
                field: "redirect_uris",
                message: "at least one redirect_uri is required".to_string(),
            });
        }
        for gt in &self.grant_types {
            if !ALLOWED_GRANT_TYPES.contains(&gt.as_str()) {
                return Err(ValidationError {
                    field: "grant_types",
                    message: format!("unsupported grant_type: {gt}"),
                });
            }
        }
        for rt in &self.response_types {
            if !ALLOWED_RESPONSE_TYPES.contains(&rt.as_str()) {
                return Err(ValidationError {
                    field: "response_types",
                    message: format!("unsupported response_type: {rt}"),
                });
            }
        }
        if !ALLOWED_TOKEN_ENDPOINT_AUTH_METHODS.contains(&self.token_endpoint_auth_method.as_str())
        {
            return Err(ValidationError {
                field: "token_endpoint_auth_method",
                message: format!(
                    "unsupported token_endpoint_auth_method: {}",
                    self.token_endpoint_auth_method
                ),
            });
        }
        Ok(())
    }
}

/// Dynamic client registration response body (RFC 7591 §3.2.1).
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DcrRegistrationResponse {
    pub client_id: String,
    pub client_id_issued_at: i64,
    pub registration_access_token: String,
    pub registration_client_uri: String,
    pub client_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_uri: Option<String>,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub scope: String,
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    #[test]
    fn dcr_request_rejects_empty_redirect_uris() {
        let req = DcrRegistrationRequest {
            client_name: "test".into(),
            client_uri: None,
            logo_uri: None,
            redirect_uris: vec![],
            grant_types: vec!["authorization_code".into()],
            response_types: vec!["code".into()],
            token_endpoint_auth_method: "none".into(),
            scope: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn dcr_request_rejects_unknown_grant_type() {
        let req = DcrRegistrationRequest {
            client_name: "test".into(),
            client_uri: None,
            logo_uri: None,
            redirect_uris: vec!["https://x/cb".into()],
            grant_types: vec!["password".into()],
            response_types: vec!["code".into()],
            token_endpoint_auth_method: "none".into(),
            scope: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn dcr_request_rejects_unknown_response_type() {
        let req = DcrRegistrationRequest {
            client_name: "test".into(),
            client_uri: None,
            logo_uri: None,
            redirect_uris: vec!["https://x/cb".into()],
            grant_types: vec!["authorization_code".into()],
            response_types: vec!["token".into()],
            token_endpoint_auth_method: "none".into(),
            scope: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn dcr_request_rejects_unknown_token_endpoint_auth_method() {
        let req = DcrRegistrationRequest {
            client_name: "test".into(),
            client_uri: None,
            logo_uri: None,
            redirect_uris: vec!["https://x/cb".into()],
            grant_types: vec!["authorization_code".into()],
            response_types: vec!["code".into()],
            token_endpoint_auth_method: "private_key_jwt".into(),
            scope: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn dcr_valid_request_passes() {
        let req = DcrRegistrationRequest {
            client_name: "test".into(),
            client_uri: None,
            logo_uri: None,
            redirect_uris: vec!["https://x/cb".into()],
            grant_types: vec!["authorization_code".into(), "refresh_token".into()],
            response_types: vec!["code".into()],
            token_endpoint_auth_method: "none".into(),
            scope: None,
        };
        assert!(req.validate().is_ok());
    }
}
