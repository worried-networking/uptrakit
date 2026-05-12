//! HTTP request types for OAuth AS endpoints. All implement
//! [`Validate`](crate::Validate) per the project rule:
//! "All HTTP request types in `uptrakit-web-api-types` implement `Validate`".
//!
//! Wire spec references: design doc §5.1 (authorization request),
//! §10.3 (token request grant variants), §12.1 (consent decision).

use serde::{Deserialize, Serialize};

use crate::validation::{Validate, ValidationError};

/// OAuth 2.1 authorization request (`GET /oauth/authorize` query params).
///
/// Per spec §5.1, `response_type` must be `"code"` and PKCE is mandatory with
/// `code_challenge_method = "S256"`. `resource` carries the RFC 8707 audience
/// indicator.
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizeRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub state: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub resource: String,
}

impl Validate for AuthorizeRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.response_type != "code" {
            return Err(ValidationError {
                field: "response_type",
                message: "response_type must be 'code'".to_string(),
            });
        }
        if self.code_challenge_method != "S256" {
            return Err(ValidationError {
                field: "code_challenge_method",
                message: "code_challenge_method must be 'S256'".to_string(),
            });
        }
        if self.code_challenge.is_empty() {
            return Err(ValidationError {
                field: "code_challenge",
                message: "code_challenge is required (PKCE)".to_string(),
            });
        }
        if self.state.is_empty() {
            return Err(ValidationError {
                field: "state",
                message: "state is required".to_string(),
            });
        }
        if self.resource.is_empty() {
            return Err(ValidationError {
                field: "resource",
                message: "resource indicator is required (RFC 8707)".to_string(),
            });
        }
        Ok(())
    }
}

/// OAuth 2.1 token request body (`POST /oauth/token`).
///
/// Internally tagged on `grant_type` so axum's form-encoded extractor (or any
/// `serde` deserialiser) picks the correct variant from a flat body.
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "grant_type", rename_all = "snake_case")]
pub enum TokenRequest {
    AuthorizationCode {
        code: String,
        redirect_uri: String,
        client_id: String,
        code_verifier: String,
        resource: String,
    },
    RefreshToken {
        refresh_token: String,
        client_id: String,
        #[serde(default)]
        scope: Option<String>,
        resource: String,
    },
}

impl Validate for TokenRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            TokenRequest::AuthorizationCode {
                code,
                code_verifier,
                resource,
                ..
            } => {
                if code.is_empty() {
                    return Err(ValidationError {
                        field: "code",
                        message: "code is required".to_string(),
                    });
                }
                if code_verifier.is_empty() {
                    return Err(ValidationError {
                        field: "code_verifier",
                        message: "code_verifier is required (PKCE)".to_string(),
                    });
                }
                if resource.is_empty() {
                    return Err(ValidationError {
                        field: "resource",
                        message: "resource indicator is required (RFC 8707)".to_string(),
                    });
                }
                Ok(())
            }
            TokenRequest::RefreshToken {
                refresh_token,
                resource,
                ..
            } => {
                if refresh_token.is_empty() {
                    return Err(ValidationError {
                        field: "refresh_token",
                        message: "refresh_token is required".to_string(),
                    });
                }
                if resource.is_empty() {
                    return Err(ValidationError {
                        field: "resource",
                        message: "resource indicator is required (RFC 8707)".to_string(),
                    });
                }
                Ok(())
            }
        }
    }
}

/// User-driven consent decision posted from the consent UI.
///
/// `typed_confirmation` carries the hostname the user typed in the
/// "unverified client" confirmation flow; it is required only when the
/// targeted client's `trusted_at` is null (spec §12.1). Higher-layer policy —
/// not this struct — enforces the requirement, so this `Validate` impl is a
/// no-op placeholder that keeps the type aligned with the project rule.
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsentDecision {
    /// Hostname the user typed for unverified-client confirmation. Required
    /// when the client's `trusted_at` is `NULL`; checked outside this struct.
    pub typed_confirmation: Option<String>,
}

impl Validate for ConsentDecision {
    fn validate(&self) -> Result<(), ValidationError> {
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

    fn valid_authorize() -> AuthorizeRequest {
        AuthorizeRequest {
            response_type: "code".into(),
            client_id: "x".into(),
            redirect_uri: "https://x/cb".into(),
            scope: "mcp:read".into(),
            state: "s".into(),
            code_challenge: "c".into(),
            code_challenge_method: "S256".into(),
            resource: "https://x/mcp".into(),
        }
    }

    #[test]
    fn valid_authorize_passes() {
        assert!(valid_authorize().validate().is_ok());
    }

    #[test]
    fn authorize_request_validates_response_type() {
        let mut req = valid_authorize();
        req.response_type = "token".into();
        assert!(req.validate().is_err());
    }

    #[test]
    fn authorize_request_requires_s256() {
        let mut req = valid_authorize();
        req.code_challenge_method = "plain".into();
        assert!(req.validate().is_err());
    }

    #[test]
    fn authorize_request_requires_code_challenge() {
        let mut req = valid_authorize();
        req.code_challenge = String::new();
        assert!(req.validate().is_err());
    }

    #[test]
    fn authorize_request_requires_state() {
        let mut req = valid_authorize();
        req.state = String::new();
        assert!(req.validate().is_err());
    }

    #[test]
    fn authorize_request_requires_resource() {
        let mut req = valid_authorize();
        req.resource = String::new();
        assert!(req.validate().is_err());
    }

    #[test]
    fn token_request_authorization_code_validates() {
        let r = TokenRequest::AuthorizationCode {
            code: "c".into(),
            redirect_uri: "https://x/cb".into(),
            client_id: "x".into(),
            code_verifier: "v".into(),
            resource: "https://x/mcp".into(),
        };
        assert!(r.validate().is_ok());
    }

    #[test]
    fn token_request_rejects_empty_code() {
        let r = TokenRequest::AuthorizationCode {
            code: String::new(),
            redirect_uri: "https://x/cb".into(),
            client_id: "x".into(),
            code_verifier: "v".into(),
            resource: "https://x/mcp".into(),
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn token_request_refresh_rejects_empty_refresh_token() {
        let r = TokenRequest::RefreshToken {
            refresh_token: String::new(),
            client_id: "x".into(),
            scope: None,
            resource: "https://x/mcp".into(),
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn consent_decision_validates() {
        let c = ConsentDecision {
            typed_confirmation: Some("x".into()),
        };
        assert!(c.validate().is_ok());
    }
}
