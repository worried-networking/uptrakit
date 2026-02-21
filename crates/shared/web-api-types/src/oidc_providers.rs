use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use time::OffsetDateTime;
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

pub fn default_scopes() -> String {
    "openid email profile groups".to_string()
}

pub fn default_auto_create() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateOidcProviderRequest {
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: SecretString,
    #[serde(default = "default_scopes")]
    pub scopes: String,
    #[serde(default = "default_auto_create")]
    pub auto_create_users: bool,
    pub role_claim_path: Option<String>,
    #[serde(default)]
    pub role_mapping: HashMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateOidcProviderRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub logo_url: Option<String>,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub scopes: Option<String>,
    pub auto_create_users: Option<bool>,
    pub role_claim_path: Option<String>,
    pub role_mapping: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcProviderResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
    pub issuer_url: String,
    pub client_id: String,
    pub has_client_secret: bool,
    pub scopes: String,
    pub auto_create_users: bool,
    pub role_claim_path: Option<String>,
    pub role_mapping: HashMap<String, String>,
    pub is_active: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

impl Validate for CreateOidcProviderRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "must not be empty".to_string(),
            });
        }

        if self.slug.len() > 64 {
            return Err(ValidationError {
                field: "slug",
                message: "must be at most 64 characters".to_string(),
            });
        }

        let first = self.slug.as_bytes().first().copied().unwrap_or(0);
        let valid_slug = !self.slug.is_empty()
            && (first.is_ascii_lowercase() || first.is_ascii_digit())
            && self
                .slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid_slug {
            return Err(ValidationError {
                field: "slug",
                message: "must match ^[a-z0-9][a-z0-9-]*$".to_string(),
            });
        }

        if !self.issuer_url.starts_with("http://") && !self.issuer_url.starts_with("https://") {
            return Err(ValidationError {
                field: "issuer_url",
                message: "must start with http:// or https://".to_string(),
            });
        }

        if self.client_id.is_empty() {
            return Err(ValidationError {
                field: "client_id",
                message: "must not be empty".to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    fn valid_create_request() -> CreateOidcProviderRequest {
        CreateOidcProviderRequest {
            name: "Keycloak".to_string(),
            slug: "keycloak".to_string(),
            logo_url: None,
            issuer_url: "https://auth.example.com/realms/main".to_string(),
            client_id: "uptrakit".to_string(),
            client_secret: SecretString::new("super-secret".to_string()),
            scopes: default_scopes(),
            auto_create_users: true,
            role_claim_path: None,
            role_mapping: HashMap::new(),
        }
    }

    // ── CreateOidcProviderRequest ────────────────────────────────────

    #[test]
    fn create_request_round_trip() {
        let req = valid_create_request();
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: CreateOidcProviderRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.name, "Keycloak");
        assert_eq!(de.slug, "keycloak");
        assert_eq!(de.client_id, "uptrakit");
        assert_eq!(de.client_secret.expose_secret(), "super-secret");
        assert!(de.auto_create_users);
    }

    #[test]
    fn create_request_defaults() {
        let json = r#"{
            "name": "SSO",
            "slug": "sso",
            "issuer_url": "https://auth.example.com",
            "client_id": "app",
            "client_secret": "s3cr3t"
        }"#;
        let de: CreateOidcProviderRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(de.scopes, "openid email profile groups");
        assert!(de.auto_create_users);
        assert!(de.role_mapping.is_empty());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut req = valid_create_request();
        req.name = String::new();
        let err = req.validate().expect_err("should reject empty name");
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_rejects_slug_too_long() {
        let mut req = valid_create_request();
        req.slug = "a".repeat(65);
        let err = req.validate().expect_err("should reject slug > 64 chars");
        assert_eq!(err.field, "slug");
    }

    #[test]
    fn validate_rejects_invalid_slug_chars() {
        let mut req = valid_create_request();
        req.slug = "My Provider!".to_string();
        let err = req
            .validate()
            .expect_err("should reject invalid slug chars");
        assert_eq!(err.field, "slug");
    }

    #[test]
    fn validate_rejects_invalid_issuer_url() {
        let mut req = valid_create_request();
        req.issuer_url = "ftp://bad.example.com".to_string();
        let err = req.validate().expect_err("should reject ftp:// issuer_url");
        assert_eq!(err.field, "issuer_url");
    }

    #[test]
    fn validate_rejects_empty_client_id() {
        let mut req = valid_create_request();
        req.client_id = String::new();
        let err = req.validate().expect_err("should reject empty client_id");
        assert_eq!(err.field, "client_id");
    }

    #[test]
    fn validate_accepts_valid_request() {
        let req = valid_create_request();
        assert!(req.validate().is_ok());
    }

    // ── OidcProviderResponse ─────────────────────────────────────────

    #[test]
    fn response_round_trip() {
        use time::macros::datetime;
        let resp = OidcProviderResponse {
            id: sample_uuid(),
            name: "Keycloak".to_string(),
            slug: "keycloak".to_string(),
            logo_url: Some("https://example.com/logo.svg".to_string()),
            issuer_url: "https://auth.example.com/realms/main".to_string(),
            client_id: "uptrakit".to_string(),
            has_client_secret: true,
            scopes: "openid email profile".to_string(),
            auto_create_users: true,
            role_claim_path: Some("resource_access.uptrakit.roles".to_string()),
            role_mapping: HashMap::from([("admin".to_string(), "admin".to_string())]),
            is_active: true,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 00:00:00 UTC),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: OidcProviderResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.id, sample_uuid());
        assert_eq!(de.name, "Keycloak");
        assert!(de.has_client_secret);
        assert!(de.is_active);
        assert_eq!(de.role_mapping.get("admin"), Some(&"admin".to_string()));
    }
}
