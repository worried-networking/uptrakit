use serde::Deserialize;

/// Response from the OCI Distribution Spec tag list endpoint.
/// `GET /v2/{name}/tags/list`
#[derive(Debug, Clone, Deserialize)]
pub struct TagListResponse {
    /// Repository name.
    #[serde(default)]
    pub name: String,
    /// List of tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

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
    fn deserialize_tag_list() {
        let json = serde_json::json!({
            "name": "library/nginx",
            "tags": ["1.25.0", "1.25.1", "1.26.0", "latest"]
        });
        let resp: TagListResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.name, "library/nginx");
        assert_eq!(resp.tags.len(), 4);
        assert!(resp.tags.contains(&"latest".to_string()));
    }

    #[test]
    fn deserialize_tag_list_empty_tags() {
        let json = serde_json::json!({
            "name": "library/alpine",
            "tags": []
        });
        let resp: TagListResponse = serde_json::from_value(json).expect("deserialize");
        assert!(resp.tags.is_empty());
    }

    #[test]
    fn deserialize_tag_list_null_tags() {
        // Some registries may return null instead of empty array
        let json = serde_json::json!({
            "name": "library/alpine"
        });
        let resp: TagListResponse = serde_json::from_value(json).expect("deserialize");
        assert!(resp.tags.is_empty());
    }

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
    fn deserialize_tag_list_with_extra_fields() {
        let json = serde_json::json!({
            "name": "library/nginx",
            "tags": ["1.0"],
            "extra_field": "ignored"
        });
        let resp: TagListResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.tags.len(), 1);
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
