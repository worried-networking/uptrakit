use serde::{Deserialize, Serialize};
use uptrakit_shared_types::SecretString;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcProviderInfo {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub logo_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthMethodsResponse {
    pub password: bool,
    pub oidc_providers: Vec<OidcProviderInfo>,
    pub setup_required: bool,
    /// Whether OIDC registration requires a registration token.
    pub registration_token_required: bool,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcAuthorizeResponse {
    pub authorize_url: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcLinkRequest {
    pub link_token: SecretString,
    pub password: Option<SecretString>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcExchangeRequest {
    pub code: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcCompleteRegistrationRequest {
    pub registration_code: SecretString,
    pub registration_token: SecretString,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── OidcProviderInfo ─────────────────────────────────────────────

    #[test]
    fn oidc_provider_info_round_trip() {
        let info = OidcProviderInfo {
            id: sample_uuid(),
            name: "Keycloak".to_string(),
            slug: "keycloak".to_string(),
            logo_url: Some("https://example.com/logo.png".to_string()),
        };
        let json = serde_json::to_string(&info).expect("serialization should succeed");
        let de: OidcProviderInfo =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.id, sample_uuid());
        assert_eq!(de.name, "Keycloak");
        assert_eq!(de.slug, "keycloak");
        assert_eq!(de.logo_url.as_deref(), Some("https://example.com/logo.png"));
    }

    // ── AuthMethodsResponse ──────────────────────────────────────────

    #[test]
    fn auth_methods_response_round_trip() {
        let resp = AuthMethodsResponse {
            password: true,
            oidc_providers: vec![OidcProviderInfo {
                id: sample_uuid(),
                name: "SSO".to_string(),
                slug: "sso".to_string(),
                logo_url: None,
            }],
            setup_required: false,
            registration_token_required: true,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let de: AuthMethodsResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(de.password);
        assert_eq!(de.oidc_providers.len(), 1);
        assert!(!de.setup_required);
        assert!(de.registration_token_required);
    }

    // ── OidcLinkRequest ──────────────────────────────────────────────

    #[test]
    fn oidc_link_request_secret_string_round_trip() {
        let req = OidcLinkRequest {
            link_token: SecretString::new("tok-abc"),
            password: Some(SecretString::new("pass123")),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: OidcLinkRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.link_token.expose_secret(), "tok-abc");
        assert_eq!(
            de.password.as_ref().map(|s| s.expose_secret()),
            Some("pass123")
        );
    }

    // ── OidcCompleteRegistrationRequest ──────────────────────────────

    #[test]
    fn oidc_complete_registration_request_round_trip() {
        let req = OidcCompleteRegistrationRequest {
            registration_code: SecretString::new("code-xyz"),
            registration_token: SecretString::new("token-123"),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: OidcCompleteRegistrationRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.registration_code.expose_secret(), "code-xyz");
        assert_eq!(de.registration_token.expose_secret(), "token-123");
    }

    // ── OidcExchangeRequest ──────────────────────────────────────────

    #[test]
    fn oidc_exchange_request_round_trip() {
        let req = OidcExchangeRequest {
            code: "auth-code-123".to_string(),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let de: OidcExchangeRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.code, "auth-code-123");
    }
}
