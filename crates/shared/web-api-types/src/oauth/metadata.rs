//! RFC 8414 Authorization Server Metadata + RFC 9728 Protected Resource
//! Metadata types.
//!
//! Both are pure wire response types — no `Validate` impl is needed since
//! they are server-produced and never deserialised from untrusted input.

use serde::{Deserialize, Serialize};

/// OAuth 2.0 Authorization Server Metadata (RFC 8414 §2) returned from
/// `/.well-known/oauth-authorization-server`.
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    pub scopes_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub client_id_metadata_document_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_documentation: Option<String>,
}

/// OAuth 2.0 Protected Resource Metadata (RFC 9728 §2) returned from
/// `/.well-known/oauth-protected-resource`.
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Vec<String>,
    pub bearer_methods_supported: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_documentation: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_metadata_includes_required_fields() {
        let meta = AuthorizationServerMetadata {
            issuer: "https://controller.example.com".into(),
            authorization_endpoint: "https://controller.example.com/oauth/authorize".into(),
            token_endpoint: "https://controller.example.com/oauth/token".into(),
            registration_endpoint: Some("https://controller.example.com/oauth/register".into()),
            scopes_supported: vec!["mcp:read".into(), "mcp:write".into()],
            response_types_supported: vec!["code".into()],
            grant_types_supported: vec!["authorization_code".into(), "refresh_token".into()],
            code_challenge_methods_supported: vec!["S256".into()],
            token_endpoint_auth_methods_supported: vec![
                "none".into(),
                "client_secret_basic".into(),
            ],
            client_id_metadata_document_supported: true,
            service_documentation: Some("https://controller.example.com/docs/oauth".into()),
        };
        let json = serde_json::to_value(&meta).expect("serialises");
        assert_eq!(
            json["code_challenge_methods_supported"],
            serde_json::json!(["S256"])
        );
    }

    #[test]
    fn prm_includes_authorization_servers_array() {
        let prm = ProtectedResourceMetadata {
            resource: "https://controller.example.com/mcp".into(),
            authorization_servers: vec!["https://controller.example.com".into()],
            scopes_supported: vec!["mcp:read".into(), "mcp:write".into()],
            bearer_methods_supported: vec!["header".into()],
            resource_documentation: Some("https://controller.example.com/docs/mcp".into()),
        };
        let json = serde_json::to_value(&prm).expect("serialises");
        assert!(json["authorization_servers"].is_array());
    }
}
