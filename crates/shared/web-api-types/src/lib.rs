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

#[cfg(test)]
mod tests {
    use crate::agents::{AgentResponse, AgentStatus};
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
        assert_eq!(Permission::iter().count(), 5);
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

    // ── 2. AgentStatus enum round-trip ────────────────────────────────────

    #[test]
    fn agent_status_serde_round_trip() {
        let variants = [
            AgentStatus::Pending,
            AgentStatus::Approved,
            AgentStatus::Rejected,
        ];
        for status in &variants {
            let json = serde_json::to_string(status).unwrap();
            let deserialized: AgentStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(&deserialized, status);
        }
    }

    #[test]
    fn agent_status_as_str_values() {
        assert_eq!(AgentStatus::Pending.as_str(), "pending");
        assert_eq!(AgentStatus::Approved.as_str(), "approved");
        assert_eq!(AgentStatus::Rejected.as_str(), "rejected");
    }

    #[test]
    fn agent_status_from_str_valid() {
        assert_eq!(
            "pending".parse::<AgentStatus>().ok(),
            Some(AgentStatus::Pending)
        );
        assert_eq!(
            "approved".parse::<AgentStatus>().ok(),
            Some(AgentStatus::Approved)
        );
        assert_eq!(
            "rejected".parse::<AgentStatus>().ok(),
            Some(AgentStatus::Rejected)
        );
    }

    #[test]
    fn agent_status_from_str_invalid_returns_none() {
        assert!("unknown".parse::<AgentStatus>().is_err());
        assert!("".parse::<AgentStatus>().is_err());
        assert!("PENDING".parse::<AgentStatus>().is_err());
    }

    #[test]
    fn agent_status_as_str_round_trips_through_from_str() {
        let variants = [
            AgentStatus::Pending,
            AgentStatus::Approved,
            AgentStatus::Rejected,
        ];
        for status in &variants {
            let s = status.as_str();
            let parsed: AgentStatus = s
                .parse()
                .expect("from_str should succeed for as_str output");
            assert_eq!(&parsed, status);
        }
    }

    // ── 3. RegistrationMode enum round-trip ───────────────────────────────

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
            "provider_config_id": "some-uuid"
        });
        let req: CreateSoftwareItemRequest = serde_json::from_value(json).unwrap();
        assert!(req.enabled);
    }

    #[test]
    fn create_software_item_request_explicit_enabled_false() {
        let json = serde_json::json!({
            "name": "Node.js",
            "provider_config_id": "some-uuid",
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

    // ── 5. skip_serializing_if ────────────────────────────────────────────

    #[test]
    fn device_auth_poll_response_omits_none_fields() {
        let resp = DeviceAuthPollResponse {
            status: "authorization_pending".to_string(),
            token: None,
            token_name: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("status"));
        assert!(
            !obj.contains_key("token"),
            "token should be omitted when None"
        );
        assert!(
            !obj.contains_key("token_name"),
            "token_name should be omitted when None"
        );
    }

    #[test]
    fn device_auth_poll_response_includes_some_fields() {
        let resp = DeviceAuthPollResponse {
            status: "complete".to_string(),
            token: Some("secret-token-value".to_string()),
            token_name: Some("my-device".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        let obj = json.as_object().unwrap();

        assert_eq!(obj.get("status").unwrap(), "complete");
        assert_eq!(obj.get("token").unwrap(), "secret-token-value");
        assert_eq!(obj.get("token_name").unwrap(), "my-device");
    }

    #[test]
    fn device_auth_poll_response_round_trip_with_none() {
        let resp = DeviceAuthPollResponse {
            status: "authorization_pending".to_string(),
            token: None,
            token_name: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: DeviceAuthPollResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, "authorization_pending");
        assert!(deserialized.token.is_none());
        assert!(deserialized.token_name.is_none());
    }

    #[test]
    fn device_auth_poll_response_round_trip_with_some() {
        let resp = DeviceAuthPollResponse {
            status: "complete".to_string(),
            token: Some("tok".to_string()),
            token_name: Some("dev".to_string()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: DeviceAuthPollResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, "complete");
        assert_eq!(deserialized.token.as_deref(), Some("tok"));
        assert_eq!(deserialized.token_name.as_deref(), Some("dev"));
    }

    // ── 6. Representative struct round-trips ──────────────────────────────

    #[test]
    fn user_response_round_trip() {
        let user = UserResponse {
            id: "usr-001".to_string(),
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

        assert_eq!(deserialized.id, "usr-001");
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
        let user = UserResponse {
            id: "usr-002".to_string(),
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
        let auth = AuthResponse {
            access_token: "eyJhbGciOiJIUzI1NiJ9.test".to_string(),
            refresh_token: "refresh-token-value".to_string(),
            expires_in: 3600,
            token_type: "Bearer".to_string(),
            user: UserResponse {
                id: "usr-001".to_string(),
                email: "admin@example.com".to_string(),
                first_name: "Admin".to_string(),
                last_name: "User".to_string(),
                permissions: vec![Permission::ViewSettings, Permission::ManageAgents],
            },
        };
        let json = serde_json::to_string(&auth).unwrap();
        let deserialized: AuthResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.access_token, "eyJhbGciOiJIUzI1NiJ9.test");
        assert_eq!(deserialized.refresh_token, "refresh-token-value");
        assert_eq!(deserialized.expires_in, 3600);
        assert_eq!(deserialized.token_type, "Bearer");
        assert_eq!(deserialized.user.id, "usr-001");
        assert_eq!(deserialized.user.email, "admin@example.com");
        assert_eq!(deserialized.user.permissions.len(), 2);
        assert_eq!(deserialized.user.permissions[0], Permission::ViewSettings);
        assert_eq!(deserialized.user.permissions[1], Permission::ManageAgents);
    }

    #[test]
    fn agent_response_round_trip() {
        let agent = AgentResponse {
            id: "agent-001".to_string(),
            hostname: "server-1.local".to_string(),
            friendly_name: "Production Server 1".to_string(),
            ip_address: Some("192.168.1.10".to_string()),
            status: AgentStatus::Approved,
            agent_version: "0.0.1".to_string(),
            last_seen_at: Some("2025-01-15T10:30:00Z".to_string()),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-15T10:30:00Z".to_string(),
        };
        let json = serde_json::to_string(&agent).unwrap();
        let deserialized: AgentResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "agent-001");
        assert_eq!(deserialized.hostname, "server-1.local");
        assert_eq!(deserialized.friendly_name, "Production Server 1");
        assert_eq!(deserialized.ip_address.as_deref(), Some("192.168.1.10"));
        assert_eq!(deserialized.status, AgentStatus::Approved);
        assert_eq!(deserialized.agent_version, "0.0.1");
        assert_eq!(
            deserialized.last_seen_at.as_deref(),
            Some("2025-01-15T10:30:00Z")
        );
        assert_eq!(deserialized.created_at, "2025-01-01T00:00:00Z");
        assert_eq!(deserialized.updated_at, "2025-01-15T10:30:00Z");
    }

    #[test]
    fn agent_response_round_trip_with_none_optionals() {
        let agent = AgentResponse {
            id: "agent-002".to_string(),
            hostname: "server-2.local".to_string(),
            friendly_name: "Staging Server".to_string(),
            ip_address: None,
            status: AgentStatus::Pending,
            agent_version: "unknown".to_string(),
            last_seen_at: None,
            created_at: "2025-02-01T00:00:00Z".to_string(),
            updated_at: "2025-02-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&agent).unwrap();
        let deserialized: AgentResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "agent-002");
        assert!(deserialized.ip_address.is_none());
        assert_eq!(deserialized.status, AgentStatus::Pending);
        assert_eq!(deserialized.agent_version, "unknown");
        assert!(deserialized.last_seen_at.is_none());
    }

    // ── 7. UpdateStatus enum round-trip ──────────────────────────────────

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
            !obj.contains_key("code"),
            "code should be omitted when None"
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

        // Agents / Services
        let _ = AgentStatus::Pending;
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
}
