use serde::Deserialize;

/// Response from the registry token endpoint (Bearer token auth).
///
/// Some registries (e.g. Docker Hub) return **both** `token` and
/// `access_token` with the same value. Serde's `alias` attribute errors on
/// duplicate keys, so both fields are modelled as optional; the caller picks
/// whichever is populated.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    /// Bearer token (`token` key).
    pub token: Option<String>,
    /// Bearer token (`access_token` key — OAuth 2.0 style).
    pub access_token: Option<String>,
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

/// OCI Image Index / Docker Manifest List entry list.
#[derive(Debug, Deserialize)]
pub struct OciManifestIndex {
    pub manifests: Vec<OciManifestEntry>,
}

/// A single entry in an OCI Image Index or Docker Manifest List.
#[derive(Debug, Deserialize)]
pub struct OciManifestEntry {
    pub digest: String,
    pub platform: Option<OciPlatform>,
}

/// Platform descriptor from an OCI manifest index entry.
#[derive(Debug, Deserialize)]
pub struct OciPlatform {
    pub architecture: String,
    pub os: String,
    pub variant: Option<String>,
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
        assert!(resp.token.as_deref().unwrap_or("").starts_with("eyJ"));
        assert_eq!(resp.expires_in, Some(300));
    }

    #[test]
    fn deserialize_token_response_with_access_token_only() {
        // Some registries use only "access_token" (OAuth 2.0 style).
        let json = serde_json::json!({
            "access_token": "abc123",
            "expires_in": 600
        });
        let resp: TokenResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.access_token.as_deref(), Some("abc123"));
        assert!(resp.token.is_none());
    }

    #[test]
    fn deserialize_token_response_both_keys() {
        // Docker Hub returns both "token" and "access_token" with the same value.
        // With separate optional fields this must not error on duplicate keys.
        let json = serde_json::json!({
            "token": "tok1",
            "access_token": "tok1",
            "expires_in": 300,
            "issued_at": "2026-03-10T20:16:49Z"
        });
        let resp: TokenResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.token.as_deref(), Some("tok1"));
        assert_eq!(resp.access_token.as_deref(), Some("tok1"));
    }

    #[test]
    fn deserialize_token_response_no_expiry() {
        let json = serde_json::json!({
            "token": "abc"
        });
        let resp: TokenResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.token.as_deref(), Some("abc"));
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
    fn deserialize_oci_manifest_index() {
        let json = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "digest": "sha256:amd64",
                    "platform": {"os": "linux", "architecture": "amd64"}
                },
                {
                    "digest": "sha256:arm64",
                    "platform": {"os": "linux", "architecture": "arm64", "variant": "v8"}
                },
                {
                    "digest": "sha256:armv7",
                    "platform": {"os": "linux", "architecture": "arm", "variant": "v7"}
                }
            ]
        });
        let idx: OciManifestIndex = serde_json::from_value(json).expect("deserialize");
        assert_eq!(idx.manifests.len(), 3);
        let armv7 = idx
            .manifests
            .iter()
            .find(|e| e.digest == "sha256:armv7")
            .unwrap();
        let p = armv7.platform.as_ref().unwrap();
        assert_eq!(p.os, "linux");
        assert_eq!(p.architecture, "arm");
        assert_eq!(p.variant.as_deref(), Some("v7"));
    }

    #[test]
    fn deserialize_oci_manifest_entry_no_platform() {
        // attestation entries often have no platform field
        let json = serde_json::json!({"digest": "sha256:attest", "mediaType": "application/vnd.oci.image.manifest.v1+json"});
        let entry: OciManifestEntry = serde_json::from_value(json).expect("deserialize");
        assert!(entry.platform.is_none());
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
        assert_eq!(resp.token.as_deref(), Some("tok"));
    }
}
