use serde::Deserialize;

/// Response from the registry token endpoint (Bearer token auth).
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    /// The bearer token.
    #[serde(alias = "access_token")]
    pub token: String,
    /// Token expiry in seconds (optional).
    pub expires_in: Option<u64>,
}

/// OCI Distribution API error response.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryErrorResponse {
    #[serde(default)]
    pub errors: Vec<RegistryError>,
}

/// A single error entry from the registry.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryError {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_token_response() {
        let json = serde_json::json!({
            "token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
            "expires_in": 300
        });
        let resp: TokenResponse = serde_json::from_value(json).expect("deserialize");
        assert!(resp.token.starts_with("eyJ"));
        assert_eq!(resp.expires_in, Some(300));
    }

    #[test]
    fn deserialize_token_response_with_access_token() {
        // Some registries use "access_token" instead of "token"
        let json = serde_json::json!({
            "access_token": "abc123",
            "expires_in": 600
        });
        let resp: TokenResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.token, "abc123");
    }

    #[test]
    fn deserialize_token_response_no_expiry() {
        let json = serde_json::json!({
            "token": "abc"
        });
        let resp: TokenResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.token, "abc");
        assert!(resp.expires_in.is_none());
    }

    #[test]
    fn deserialize_registry_error() {
        let json = serde_json::json!({
            "errors": [
                {
                    "code": "UNAUTHORIZED",
                    "message": "authentication required"
                }
            ]
        });
        let resp: RegistryErrorResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.errors.len(), 1);
        assert_eq!(resp.errors[0].code, "UNAUTHORIZED");
        assert_eq!(resp.errors[0].message, "authentication required");
    }

    #[test]
    fn deserialize_registry_error_multiple() {
        let json = serde_json::json!({
            "errors": [
                {"code": "DENIED", "message": "access denied"},
                {"code": "NAME_UNKNOWN", "message": "repository not found"}
            ]
        });
        let resp: RegistryErrorResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.errors.len(), 2);
    }

    #[test]
    fn deserialize_token_response_with_extra_fields() {
        let json = serde_json::json!({
            "token": "tok",
            "expires_in": 60,
            "issued_at": "2024-01-01T00:00:00Z",
            "scope": "repository:library/nginx:pull"
        });
        let resp: TokenResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.token, "tok");
    }
}
