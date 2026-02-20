/// Re-export `SecretString` so consumers (web-api, CLI, openapi-client) can
/// use it without a direct `uptrakit-shared-types` dependency.
pub use uptrakit_shared_types::SecretString;

pub mod agents;
pub mod api_tokens;
pub mod auth;
pub mod device_auth;
pub mod error;
pub mod hosts;
pub mod mqtt_services;
pub mod mqtt_transport;
pub mod mqtt_url;
pub mod oidc_auth;
pub mod oidc_providers;
pub mod pagination;
pub mod permissions;
pub mod prelude;
pub mod provider_configs;
pub mod registration;
pub mod scheduler;
pub mod server_cert;
pub mod services;
pub mod settings;
pub mod settings_agent_certs;
pub mod settings_auth;
pub mod settings_ca;
pub mod settings_combined;
pub mod settings_mqtt;
pub mod settings_network;
pub mod software_items;
pub mod system_alerts;
pub mod update_history;
pub mod update_hooks;
pub mod validation;

/// Default value for `enabled` fields in create-request types.
///
/// Used as `#[serde(default = "crate::default_enabled")]` in
/// [`provider_configs::CreateProviderConfigRequest`] and
/// [`software_items::CreateSoftwareItemRequest`].
pub fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use crate::auth::{AuthResponse, UserResponse};
    use crate::device_auth::DeviceAuthPollResponse;
    use crate::error::ErrorResponse;
    use crate::oidc_providers::CreateOidcProviderRequest;
    use crate::permissions::Permission;
    use crate::provider_configs::CreateProviderConfigRequest;
    use crate::registration::RegistrationMode;
    use crate::software_items::CreateSoftwareItemRequest;
    use crate::update_history::UpdateStatus;
    use strum::IntoEnumIterator;
    use uptrakit_shared_types::{DeviceAuthStatus, SecretString};
    use uuid::Uuid;

    // ── 1. Permission enum round-trip ─────────────────────────────────────

    #[test]
    fn permission_serde_round_trip() {
        for perm in Permission::iter() {
            let json = serde_json::to_string(&perm).unwrap();
            let deserialized: Permission = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, perm);
        }
    }

    #[test]
    fn permission_as_str_values() {
        assert_eq!(Permission::ViewSettings.as_str(), "view_settings");
        assert_eq!(Permission::ManageSettings.as_str(), "manage_settings");
        assert_eq!(Permission::ViewAgents.as_str(), "view_agents");
        assert_eq!(Permission::ManageAgents.as_str(), "manage_agents");
        assert_eq!(
            Permission::ManageGlobalSettings.as_str(),
            "manage_global_settings"
        );
    }

    #[test]
    fn permission_from_str_valid() {
        assert_eq!(
            "view_settings".parse::<Permission>().ok(),
            Some(Permission::ViewSettings)
        );
        assert_eq!(
            "manage_settings".parse::<Permission>().ok(),
            Some(Permission::ManageSettings)
        );
        assert_eq!(
            "view_agents".parse::<Permission>().ok(),
            Some(Permission::ViewAgents)
        );
        assert_eq!(
            "manage_agents".parse::<Permission>().ok(),
            Some(Permission::ManageAgents)
        );
        assert_eq!(
            "manage_global_settings".parse::<Permission>().ok(),
            Some(Permission::ManageGlobalSettings)
        );
    }

    #[test]
    fn permission_from_str_invalid_returns_err() {
        assert!("nonexistent".parse::<Permission>().is_err());
        assert!("".parse::<Permission>().is_err());
        assert!("VIEW_SETTINGS".parse::<Permission>().is_err());
    }

    #[test]
    fn permission_display_matches_as_str() {
        for perm in Permission::iter() {
            assert_eq!(format!("{perm}"), perm.as_str());
        }
    }

    #[test]
    fn permission_iter_covers_all_variants() {
        assert_eq!(Permission::iter().count(), 9);
    }

    #[test]
    fn permission_as_str_round_trips_through_from_str() {
        for perm in Permission::iter() {
            let s = perm.as_str();
            let parsed: Permission = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(parsed, perm);
        }
    }

    // ── 2. RegistrationMode enum round-trip ──────────────────────────────

    #[test]
    fn registration_mode_serde_round_trip() {
        let variants = [
            RegistrationMode::Open,
            RegistrationMode::Invite,
            RegistrationMode::Closed,
        ];
        for mode in &variants {
            let json = serde_json::to_string(mode).unwrap();
            let deserialized: RegistrationMode = serde_json::from_str(&json).unwrap();
            assert_eq!(&deserialized, mode);
        }
    }

    #[test]
    fn registration_mode_as_str_values() {
        assert_eq!(RegistrationMode::Open.as_str(), "open");
        assert_eq!(RegistrationMode::Invite.as_str(), "invite");
        assert_eq!(RegistrationMode::Closed.as_str(), "closed");
    }

    #[test]
    fn registration_mode_from_str_valid() {
        assert_eq!(
            "open".parse::<RegistrationMode>().ok(),
            Some(RegistrationMode::Open)
        );
        assert_eq!(
            "invite".parse::<RegistrationMode>().ok(),
            Some(RegistrationMode::Invite)
        );
        assert_eq!(
            "closed".parse::<RegistrationMode>().ok(),
            Some(RegistrationMode::Closed)
        );
    }

    #[test]
    fn registration_mode_from_str_invalid_returns_none() {
        assert!("disabled".parse::<RegistrationMode>().is_err());
        assert!("".parse::<RegistrationMode>().is_err());
        assert!("OPEN".parse::<RegistrationMode>().is_err());
    }

    #[test]
    fn registration_mode_as_str_round_trips_through_from_str() {
        let variants = [
            RegistrationMode::Open,
            RegistrationMode::Invite,
            RegistrationMode::Closed,
        ];
        for mode in &variants {
            let s = mode.as_str();
            let parsed: RegistrationMode = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(&parsed, mode);
        }
    }

    #[test]
    fn registration_mode_display_matches_as_str() {
        let variants = [
            RegistrationMode::Open,
            RegistrationMode::Invite,
            RegistrationMode::Closed,
        ];
        for mode in &variants {
            assert_eq!(format!("{mode}"), mode.as_str());
        }
    }

    // ── 4. Serde defaults ─────────────────────────────────────────────────

    #[test]
    fn create_oidc_provider_request_default_scopes() {
        let json = serde_json::json!({
            "name": "Test Provider",
            "slug": "test",
            "issuer_url": "https://issuer.example.com",
            "client_id": "client-id",
            "client_secret": "client-secret"
        });
        let req: CreateOidcProviderRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.scopes, "openid email profile groups");
    }

    #[test]
    fn create_oidc_provider_request_default_auto_create() {
        let json = serde_json::json!({
            "name": "Test Provider",
            "slug": "test",
            "issuer_url": "https://issuer.example.com",
            "client_id": "client-id",
            "client_secret": "client-secret"
        });
        let req: CreateOidcProviderRequest = serde_json::from_value(json).unwrap();
        assert!(req.auto_create_users);
    }

    #[test]
    fn create_oidc_provider_request_explicit_overrides_defaults() {
        let json = serde_json::json!({
            "name": "Test Provider",
            "slug": "test",
            "issuer_url": "https://issuer.example.com",
            "client_id": "client-id",
            "client_secret": "client-secret",
            "scopes": "openid",
            "auto_create_users": false
        });
        let req: CreateOidcProviderRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.scopes, "openid");
        assert!(!req.auto_create_users);
    }

    #[test]
    fn create_software_item_request_default_enabled() {
        let json = serde_json::json!({
            "name": "Node.js",
            "provider_config_id": "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
        });
        let req: CreateSoftwareItemRequest = serde_json::from_value(json).unwrap();
        assert!(req.enabled);
    }

    #[test]
    fn create_software_item_request_explicit_enabled_false() {
        let json = serde_json::json!({
            "name": "Node.js",
            "provider_config_id": "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6",
            "enabled": false
        });
        let req: CreateSoftwareItemRequest = serde_json::from_value(json).unwrap();
        assert!(!req.enabled);
    }

    #[test]
    fn create_software_item_request_inline_config_default_enabled() -> Result<(), serde_json::Error>
    {
        let json = serde_json::json!({
            "name": "Node.js",
            "provider_config": {
                "name": "GitHub Releases",
                "provider_type": "github_releases",
                "config": {}
            }
        });
        let req: CreateSoftwareItemRequest = serde_json::from_value(json)?;
        assert!(req.enabled);
        Ok(())
    }

    #[test]
    fn create_provider_config_request_default_enabled() {
        let json = serde_json::json!({
            "name": "GitHub Releases",
            "provider_type": "github_releases",
            "config": {}
        });
        let req: CreateProviderConfigRequest = serde_json::from_value(json).unwrap();
        assert!(req.enabled);
    }

    #[test]
    fn create_provider_config_request_explicit_enabled_false() {
        let json = serde_json::json!({
            "name": "GitHub Releases",
            "provider_type": "github_releases",
            "config": {},
            "enabled": false
        });
        let req: CreateProviderConfigRequest = serde_json::from_value(json).unwrap();
        assert!(!req.enabled);
    }

    // ── 5. Option fields serialize as null ─────────────────────────────────

    #[test]
    fn device_auth_poll_response_serializes_none_as_null() {
        let resp = DeviceAuthPollResponse {
            status: DeviceAuthStatus::Pending,
            token: None,
            token_name: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("status"));
        assert!(
            obj.get("token").unwrap().is_null(),
            "token should serialize as null when None"
        );
        assert!(
            obj.get("token_name").unwrap().is_null(),
            "token_name should serialize as null when None"
        );
    }

    #[test]
    fn device_auth_poll_response_includes_some_fields() {
        let resp = DeviceAuthPollResponse {
            status: DeviceAuthStatus::Authorized,
            token: Some(SecretString::new("secret-token-value".to_string())),
            token_name: Some("my-device".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        let obj = json.as_object().unwrap();

        assert_eq!(obj.get("status").unwrap(), "authorized");
        assert_eq!(obj.get("token").unwrap(), "secret-token-value");
        assert_eq!(obj.get("token_name").unwrap(), "my-device");
    }

    #[test]
    fn device_auth_poll_response_round_trip_with_none() {
        let resp = DeviceAuthPollResponse {
            status: DeviceAuthStatus::Pending,
            token: None,
            token_name: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: DeviceAuthPollResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, DeviceAuthStatus::Pending);
        assert!(deserialized.token.is_none());
        assert!(deserialized.token_name.is_none());
    }

    #[test]
    fn device_auth_poll_response_round_trip_with_some() {
        let resp = DeviceAuthPollResponse {
            status: DeviceAuthStatus::Authorized,
            token: Some(SecretString::new("tok".to_string())),
            token_name: Some("dev".to_string()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: DeviceAuthPollResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, DeviceAuthStatus::Authorized);
        assert_eq!(
            deserialized.token.as_ref().map(|t| t.expose_secret()),
            Some("tok")
        );
        assert_eq!(deserialized.token_name.as_deref(), Some("dev"));
    }

    // ── 6. Representative struct round-trips ──────────────────────────────

    #[test]
    fn user_response_round_trip() {
        let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("valid uuid");
        let user = UserResponse {
            id: user_id,
            email: "owner@example.com".to_string(),
            first_name: "Owner".to_string(),
            last_name: "User".to_string(),
            permissions: vec![
                Permission::ViewSettings,
                Permission::ManageSettings,
                Permission::ViewAgents,
                Permission::ManageAgents,
                Permission::ManageGlobalSettings,
            ],
        };
        let json = serde_json::to_string(&user).unwrap();
        let deserialized: UserResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, user_id);
        assert_eq!(deserialized.email, "owner@example.com");
        assert_eq!(deserialized.first_name, "Owner");
        assert_eq!(deserialized.last_name, "User");
        assert_eq!(deserialized.permissions.len(), 5);
        assert_eq!(deserialized.permissions[0], Permission::ViewSettings);
        assert_eq!(deserialized.permissions[1], Permission::ManageSettings);
        assert_eq!(deserialized.permissions[2], Permission::ViewAgents);
        assert_eq!(deserialized.permissions[3], Permission::ManageAgents);
        assert_eq!(
            deserialized.permissions[4],
            Permission::ManageGlobalSettings
        );
    }

    #[test]
    fn user_response_empty_permissions() {
        let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").expect("valid uuid");
        let user = UserResponse {
            id: user_id,
            email: "viewer@example.com".to_string(),
            first_name: "View".to_string(),
            last_name: "Only".to_string(),
            permissions: vec![],
        };
        let json = serde_json::to_string(&user).unwrap();
        let deserialized: UserResponse = serde_json::from_str(&json).unwrap();

        assert!(deserialized.permissions.is_empty());
    }

    #[test]
    fn auth_response_round_trip() {
        let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("valid uuid");
        let auth = AuthResponse {
            access_token: SecretString::new("eyJhbGciOiJIUzI1NiJ9.test".to_string()),
            refresh_token: SecretString::new("refresh-token-value".to_string()),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            user: UserResponse {
                id: user_id,
                email: "admin@example.com".to_string(),
                first_name: "Admin".to_string(),
                last_name: "User".to_string(),
                permissions: vec![Permission::ViewSettings, Permission::ManageAgents],
            },
        };
        let json = serde_json::to_string(&auth).unwrap();
        let deserialized: AuthResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.access_token.expose_secret(),
            "eyJhbGciOiJIUzI1NiJ9.test"
        );
        assert_eq!(
            deserialized.refresh_token.expose_secret(),
            "refresh-token-value"
        );
        assert_eq!(deserialized.expires_in, 3600);
        assert_eq!(deserialized.token_type, "Bearer");
        assert_eq!(deserialized.user.id, user_id);
        assert_eq!(deserialized.user.email, "admin@example.com");
        assert_eq!(deserialized.user.permissions.len(), 2);
        assert_eq!(deserialized.user.permissions[0], Permission::ViewSettings);
        assert_eq!(deserialized.user.permissions[1], Permission::ManageAgents);
    }

    // ── 6. UpdateStatus enum round-trip ─────────────────────────────────

    #[test]
    fn update_status_serde_round_trip() {
        let variants = [
            UpdateStatus::Pending,
            UpdateStatus::InProgress,
            UpdateStatus::Completed,
            UpdateStatus::Failed,
        ];
        for status in &variants {
            let json = serde_json::to_string(status).unwrap();
            let deserialized: UpdateStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(&deserialized, status);
        }
    }

    #[test]
    fn update_status_as_str_values() {
        assert_eq!(UpdateStatus::Pending.as_str(), "pending");
        assert_eq!(UpdateStatus::InProgress.as_str(), "in_progress");
        assert_eq!(UpdateStatus::Completed.as_str(), "completed");
        assert_eq!(UpdateStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn update_status_from_str_valid() {
        assert_eq!(
            "pending".parse::<UpdateStatus>().ok(),
            Some(UpdateStatus::Pending)
        );
        assert_eq!(
            "in_progress".parse::<UpdateStatus>().ok(),
            Some(UpdateStatus::InProgress)
        );
        assert_eq!(
            "completed".parse::<UpdateStatus>().ok(),
            Some(UpdateStatus::Completed)
        );
        assert_eq!(
            "failed".parse::<UpdateStatus>().ok(),
            Some(UpdateStatus::Failed)
        );
    }

    #[test]
    fn update_status_from_str_invalid_returns_none() {
        assert!("unknown".parse::<UpdateStatus>().is_err());
        assert!("".parse::<UpdateStatus>().is_err());
        assert!("PENDING".parse::<UpdateStatus>().is_err());
    }

    #[test]
    fn update_status_as_str_round_trips_through_from_str() {
        let variants = [
            UpdateStatus::Pending,
            UpdateStatus::InProgress,
            UpdateStatus::Completed,
            UpdateStatus::Failed,
        ];
        for status in &variants {
            let s = status.as_str();
            let parsed: UpdateStatus = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(&parsed, status);
        }
    }

    #[test]
    fn update_status_display_matches_as_str() {
        let variants = [
            UpdateStatus::Pending,
            UpdateStatus::InProgress,
            UpdateStatus::Completed,
            UpdateStatus::Failed,
        ];
        for status in &variants {
            assert_eq!(format!("{status}"), status.as_str());
        }
    }

    // ── 8. ErrorResponse round-trip ─────────────────────────────────────

    #[test]
    fn error_response_serialization_without_code() {
        let resp = ErrorResponse {
            error: "Something went wrong".to_string(),
            code: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        let obj = json.as_object().unwrap();

        assert_eq!(obj.get("error").unwrap(), "Something went wrong");
        assert!(
            obj.get("code").unwrap().is_null(),
            "code should serialize as null when None"
        );
    }

    #[test]
    fn error_response_serialization_with_code() {
        let resp = ErrorResponse {
            error: "Not found".to_string(),
            code: Some("not_found".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        let obj = json.as_object().unwrap();

        assert_eq!(obj.get("error").unwrap(), "Not found");
        assert_eq!(obj.get("code").unwrap(), "not_found");
    }

    #[test]
    fn error_response_round_trip_without_code() {
        let resp = ErrorResponse {
            error: "Bad request".to_string(),
            code: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: ErrorResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.error, "Bad request");
        assert!(deserialized.code.is_none());
    }

    #[test]
    fn error_response_round_trip_with_code() {
        let resp = ErrorResponse {
            error: "Forbidden".to_string(),
            code: Some("insufficient_permissions".to_string()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: ErrorResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.error, "Forbidden");
        assert_eq!(
            deserialized.code.as_deref(),
            Some("insufficient_permissions")
        );
    }

    #[test]
    fn error_response_deserialization_without_code_field() {
        let json = r#"{"error":"test error"}"#;
        let resp: ErrorResponse = serde_json::from_str(json).unwrap();

        assert_eq!(resp.error, "test error");
        assert!(resp.code.is_none());
    }

    // ── 9. Pagination ────────────────────────────────────────────────────

    #[test]
    fn pagination_resolve_defaults() {
        use crate::pagination::{DEFAULT_PER_PAGE, PaginationParams};
        let params = PaginationParams {
            page: None,
            per_page: None,
        };
        let resolved = params.resolve();
        assert_eq!(resolved.page, 1);
        assert_eq!(resolved.per_page, DEFAULT_PER_PAGE);
    }

    #[test]
    fn pagination_resolve_explicit_values() {
        use crate::pagination::PaginationParams;
        let params = PaginationParams {
            page: Some(3),
            per_page: Some(50),
        };
        let resolved = params.resolve();
        assert_eq!(resolved.page, 3);
        assert_eq!(resolved.per_page, 50);
    }

    #[test]
    fn pagination_resolve_clamps_page_zero_to_one() {
        use crate::pagination::PaginationParams;
        let params = PaginationParams {
            page: Some(0),
            per_page: None,
        };
        let resolved = params.resolve();
        assert_eq!(resolved.page, 1);
    }

    #[test]
    fn pagination_resolve_clamps_per_page_zero_to_one() {
        use crate::pagination::PaginationParams;
        let params = PaginationParams {
            page: None,
            per_page: Some(0),
        };
        let resolved = params.resolve();
        assert_eq!(resolved.per_page, 1);
    }

    #[test]
    fn pagination_resolve_clamps_per_page_above_max() {
        use crate::pagination::{MAX_PER_PAGE, PaginationParams};
        let params = PaginationParams {
            page: None,
            per_page: Some(5000),
        };
        let resolved = params.resolve();
        assert_eq!(resolved.per_page, MAX_PER_PAGE);
    }

    #[test]
    fn pagination_offset_calculation() {
        use crate::pagination::ResolvedPagination;
        let p = ResolvedPagination {
            page: 1,
            per_page: 20,
        };
        assert_eq!(p.offset(), 0);

        let p = ResolvedPagination {
            page: 2,
            per_page: 20,
        };
        assert_eq!(p.offset(), 20);

        let p = ResolvedPagination {
            page: 3,
            per_page: 50,
        };
        assert_eq!(p.offset(), 100);
    }

    #[test]
    fn pagination_total_pages_calculation() {
        use crate::pagination::ResolvedPagination;
        let p = ResolvedPagination {
            page: 1,
            per_page: 20,
        };
        assert_eq!(p.total_pages(0), 0);
        assert_eq!(p.total_pages(1), 1);
        assert_eq!(p.total_pages(20), 1);
        assert_eq!(p.total_pages(21), 2);
        assert_eq!(p.total_pages(100), 5);
    }

    #[test]
    fn paginated_response_serialization_round_trip() {
        use crate::pagination::{PaginatedResponse, ResolvedPagination};
        let pagination = ResolvedPagination {
            page: 2,
            per_page: 10,
        };
        let resp = PaginatedResponse::new(vec!["a".to_string(), "b".to_string()], 25, pagination);

        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: PaginatedResponse<String> = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.items, vec!["a", "b"]);
        assert_eq!(deserialized.total, 25);
        assert_eq!(deserialized.page, 2);
        assert_eq!(deserialized.per_page, 10);
        assert_eq!(deserialized.total_pages, 3);
    }

    #[test]
    fn paginated_response_empty_items() {
        use crate::pagination::{PaginatedResponse, ResolvedPagination};
        let pagination = ResolvedPagination {
            page: 1,
            per_page: 20,
        };
        let resp = PaginatedResponse::<String>::new(vec![], 0, pagination);

        assert!(resp.items.is_empty());
        assert_eq!(resp.total, 0);
        assert_eq!(resp.total_pages, 0);
    }

    // ── 10. Prelude compile check ───────────────────────────────────────

    #[test]
    fn prelude_re_exports_resolve() {
        use crate::prelude::*;

        // Auth
        let _: AuthResponse;
        let _: UserResponse;

        // Services
        let _ = ServiceStatus::Pending;

        // Hosts
        let _: HostResponse;

        // Software items
        let _: SoftwareItemResponse;
        let _ = TriggerUpdateStatus::Pending;

        // Provider configs
        let _: ProviderConfigResponse;

        // Update history
        let _ = UpdateStatus::Pending;

        // API tokens
        let _: ApiTokenListResponse;
        let _: CreateApiTokenResponse;

        // OIDC
        let _: OidcProviderResponse;
        let _: AuthMethodsResponse;

        // MQTT
        let _ = MqttClientConnectionStatus::Online;
        let _ = MqttTransport::Tcp;

        // Common
        let _: ErrorResponse;
        let _: PaginatedResponse<String>;
        let _: PaginationParams;
        let _ = Permission::ViewSettings;
        let _ = RegistrationMode::Open;
    }

    // ── 11. Request type validation ─────────────────────────────────────

    #[test]
    fn register_request_valid() {
        use crate::auth::RegisterRequest;
        use crate::validation::Validate;
        let req = RegisterRequest {
            email: "user@example.com".to_string(),
            first_name: "John".to_string(),
            last_name: "Doe".to_string(),
            password: SecretString::new("password12345678".to_string()),
            registration_token: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn register_request_email_missing_at() {
        use crate::auth::RegisterRequest;
        use crate::validation::Validate;
        let req = RegisterRequest {
            email: "invalid-email".to_string(),
            first_name: "John".to_string(),
            last_name: "Doe".to_string(),
            password: SecretString::new("password12345678".to_string()),
            registration_token: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "email");
    }

    #[test]
    fn register_request_password_too_short() {
        use crate::auth::RegisterRequest;
        use crate::validation::Validate;
        let req = RegisterRequest {
            email: "user@example.com".to_string(),
            first_name: "John".to_string(),
            last_name: "Doe".to_string(),
            password: SecretString::new("short".to_string()),
            registration_token: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }

    #[test]
    fn register_request_empty_first_name() {
        use crate::auth::RegisterRequest;
        use crate::validation::Validate;
        let req = RegisterRequest {
            email: "user@example.com".to_string(),
            first_name: "".to_string(),
            last_name: "Doe".to_string(),
            password: SecretString::new("password12345678".to_string()),
            registration_token: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "first_name");
    }

    #[test]
    fn login_request_valid() {
        use crate::auth::LoginRequest;
        use crate::validation::Validate;
        let req = LoginRequest {
            email: "user@example.com".to_string(),
            password: SecretString::new("any-password".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn login_request_empty_password() {
        use crate::auth::LoginRequest;
        use crate::validation::Validate;
        let req = LoginRequest {
            email: "user@example.com".to_string(),
            password: SecretString::new("".to_string()),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "password");
    }

    #[test]
    fn create_oidc_provider_valid() {
        use crate::validation::Validate;
        let req = CreateOidcProviderRequest {
            name: "Test Provider".to_string(),
            slug: "test-provider".to_string(),
            logo_url: None,
            issuer_url: "https://issuer.example.com".to_string(),
            client_id: "client-id".to_string(),
            client_secret: SecretString::new("secret".to_string()),
            scopes: "openid email".to_string(),
            auto_create_users: true,
            role_claim_path: None,
            role_mapping: Default::default(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_oidc_provider_invalid_slug() {
        use crate::validation::Validate;
        let req = CreateOidcProviderRequest {
            name: "Test".to_string(),
            slug: "INVALID_SLUG".to_string(),
            logo_url: None,
            issuer_url: "https://issuer.example.com".to_string(),
            client_id: "client-id".to_string(),
            client_secret: SecretString::new("secret".to_string()),
            scopes: "openid".to_string(),
            auto_create_users: true,
            role_claim_path: None,
            role_mapping: Default::default(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "slug");
    }

    #[test]
    fn create_oidc_provider_invalid_issuer_url() {
        use crate::validation::Validate;
        let req = CreateOidcProviderRequest {
            name: "Test".to_string(),
            slug: "test".to_string(),
            logo_url: None,
            issuer_url: "ftp://issuer.example.com".to_string(),
            client_id: "client-id".to_string(),
            client_secret: SecretString::new("secret".to_string()),
            scopes: "openid".to_string(),
            auto_create_users: true,
            role_claim_path: None,
            role_mapping: Default::default(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "issuer_url");
    }

    #[test]
    fn create_software_item_valid_with_config_id() {
        use crate::validation::Validate;
        let req = CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            provider_config_id: Some(
                Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap(),
            ),
            provider_config: None,
            package_identifier: None,
            config_override: None,
            enabled: true,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_software_item_neither_config_provided() {
        use crate::validation::Validate;
        let req = CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            provider_config_id: None,
            provider_config: None,
            package_identifier: None,
            config_override: None,
            enabled: true,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "provider_config");
    }

    #[test]
    fn create_provider_config_valid() {
        use crate::validation::Validate;
        use uptrakit_shared_types::ProviderType;
        let req = CreateProviderConfigRequest {
            name: "GitHub Releases".to_string(),
            provider_type: ProviderType::GithubReleases,
            config: serde_json::json!({}),
            enabled: true,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_provider_config_empty_name() {
        use crate::validation::Validate;
        use uptrakit_shared_types::ProviderType;
        let req = CreateProviderConfigRequest {
            name: "".to_string(),
            provider_type: ProviderType::GithubReleases,
            config: serde_json::json!({}),
            enabled: true,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "name");
    }

    #[test]
    fn update_scheduled_task_valid_cron() {
        use crate::scheduler::UpdateScheduledTaskRequest;
        use crate::validation::Validate;
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("0 * * * *".to_string()),
            enabled: None,
            task_config: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_scheduled_task_invalid_cron() {
        use crate::scheduler::UpdateScheduledTaskRequest;
        use crate::validation::Validate;
        let req = UpdateScheduledTaskRequest {
            cron_expression: Some("not a cron".to_string()),
            enabled: None,
            task_config: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "cron_expression");
    }

    #[test]
    fn update_network_settings_valid() {
        use crate::settings_network::UpdateNetworkSettingsRequest;
        use crate::validation::Validate;
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: Some(vec!["10.0.0.0/8".to_string()]),
            real_ip_header: Some("X-Real-IP".to_string()),
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: Some("https://pki.example.com".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_network_settings_empty_trusted_proxy() {
        use crate::settings_network::UpdateNetworkSettingsRequest;
        use crate::validation::Validate;
        let req = UpdateNetworkSettingsRequest {
            trusted_proxies: Some(vec!["".to_string()]),
            real_ip_header: None,
            extra_sans: None,
            https_addr: None,
            forwarded_client_cert_info_header: None,
            forwarded_client_cert_pem_header: None,
            pki_addr: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "trusted_proxies");
    }
}
