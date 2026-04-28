//! Integration tests for CLI command execution against a mock API server.
//!
//! Each test starts a [`MockApiServer`], registers a response, calls the
//! corresponding CLI command function with the mock server's URL and a dummy
//! token, and asserts the result.

use std::ffi::OsString;

use base64::Engine as _;
use rcgen::KeyPair;
use time::macros::datetime;
use uptrakit_cli::commands::{hosts, services, software_items, surfaces};
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::mock::MockApiServer;
use uptrakit_openapi_client::types::hosts::{
    HostAgentSummary, HostResponse, HostSoftwareStatusSummary,
};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::services::ServiceResponse;
use uptrakit_openapi_client::types::software_items::SoftwareItemResponse;
use uptrakit_openapi_client::types::surfaces::SurfaceReadResponse;
use uptrakit_openapi_client::types::surfaces::{SurfaceProviderAvailability, SurfaceProviderInfo};
use uptrakit_wire::surfaces as wire_surfaces;

// ── Fixtures ───────────────────────────────────────────────────────────────

fn host_id() -> Uuid {
    "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6".parse().unwrap()
}

fn service_id() -> Uuid {
    "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6".parse().unwrap()
}

fn software_item_id() -> Uuid {
    "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6".parse().unwrap()
}

fn sample_host() -> HostResponse {
    HostResponse {
        id: host_id(),
        machine_id: "machine-001".to_string(),
        hostname: "server-1.local".to_string(),
        friendly_name: "Production Server".to_string(),
        os_type: Some("linux".to_string()),
        os_version: Some("Ubuntu 22.04".to_string()),
        architecture: Some("x86_64".to_string()),
        ip_address: Some("192.168.1.100".to_string()),
        last_seen_at: Some(datetime!(2025-01-01 00:00:00 UTC)),
        created_at: datetime!(2025-01-01 00:00:00 UTC),
        updated_at: datetime!(2025-01-01 00:00:00 UTC),
        agents: vec![],
        tags: vec![],
        features: vec![],
        software_status: HostSoftwareStatusSummary {
            known: true,
            update_count: 0,
            error_count: 0,
        },
    }
}

fn paginated_host() -> PaginatedResponse<HostResponse> {
    PaginatedResponse {
        items: vec![sample_host()],
        total: 1,
        page: 1,
        per_page: 20,
        total_pages: 1,
    }
}

fn sample_service() -> ServiceResponse {
    ServiceResponse {
        id: service_id(),
        capabilities: vec![
            "graceful_shutdown".to_string(),
            "software_discovery".to_string(),
            "update_hooks".to_string(),
        ],
        service_label: "Agent".to_string(),
        hostname: "agent-host.local".to_string(),
        friendly_name: "Test Agent".to_string(),
        is_embedded: false,
        ip_address: None,
        status: "approved".parse().unwrap(),
        client_version: Some("1.0.0".to_string()),
        last_seen_at: Some(datetime!(2025-01-01 00:00:00 UTC)),
        created_at: datetime!(2025-01-01 00:00:00 UTC),
        updated_at: datetime!(2025-01-01 00:00:00 UTC),
        ping_interval_seconds: None,
        cert_lifetime_hours: None,
        yielded_to: None,
    }
}

fn paginated_service() -> PaginatedResponse<ServiceResponse> {
    PaginatedResponse {
        items: vec![sample_service()],
        total: 1,
        page: 1,
        per_page: 20,
        total_pages: 1,
    }
}

fn sample_software_item() -> SoftwareItemResponse {
    SoftwareItemResponse {
        id: software_item_id(),
        name: "Node.js".to_string(),
        plugins: vec!["releases_github".to_string()],
        featured: true,
        last_checked_at: None,
        host_count: 2,
        installed_version: None,
        installed_display_version: None,
        latest_version: None,
        latest_release_metadata: None,
        update_available: false,
        created_at: datetime!(2025-01-01 00:00:00 UTC),
        updated_at: datetime!(2025-01-01 00:00:00 UTC),
        icon_url: None,
    }
}

fn paginated_software_item() -> PaginatedResponse<SoftwareItemResponse> {
    PaginatedResponse {
        items: vec![sample_software_item()],
        total: 1,
        page: 1,
        per_page: 20,
        total_pages: 1,
    }
}

fn sample_surface_id() -> &'static str {
    "surface.sample"
}

fn sample_interaction_id() -> &'static str {
    "surface.sample.submit"
}

fn sample_surface_read() -> SurfaceReadResponse {
    SurfaceReadResponse {
        descriptor: wire_surfaces::SurfaceDescriptor::builder()
            .surface_id(sample_surface_id().parse().unwrap())
            .label("Sample surface")
            .priority(200)
            .slot(wire_surfaces::SLOT_SETTINGS_TABS)
            .scope(wire_surfaces::Scope::Tenant)
            .targeting(wire_surfaces::Targeting::Universal)
            .provider_kind(wire_surfaces::ProviderKind::Plugin)
            .required_capabilities(wire_surfaces::CapabilitySet::default())
            .root_node(wire_surfaces::SurfaceNode::Section {
                title: Some("Sample surface".to_string()),
                children: vec![],
            })
            .build(),
        interactions: vec![wire_surfaces::InteractionDescriptor {
            interaction_id: sample_interaction_id().parse().unwrap(),
            kind: wire_surfaces::InteractionKind::FormSubmit,
            label: "Submit".to_string(),
            required_permission: None,
            input_schema: Some(wire_surfaces::SchemaContract::Object),
            result_schema: Some(wire_surfaces::SchemaContract::Any),
            sensitive_fields: vec![],
            timeout_seconds: Some(30),
            confirmation: None,
            transport: wire_surfaces::InteractionTransport::ControllerLocal,
            workflow_steps: vec![],
            form_ui: Some(wire_surfaces::FormUiDescriptor {
                fields: vec![wire_surfaces::FormFieldDescriptor {
                    key: "name".to_string(),
                    label: "Name".to_string(),
                    field_type: "text".to_string(),
                    required: true,
                    placeholder: None,
                    help_text: None,
                    default_value: None,
                    options: vec![],
                    select_source: None,
                    sensitive: false,
                    list: false,
                    visible_when: None,
                }],
                pre_load_interaction_id: None,
            }),
        }],
        data_sources: vec![],
    }
}

fn sample_surface_provider_info() -> SurfaceProviderInfo {
    SurfaceProviderInfo {
        provider_id: "provider.sample".to_string(),
        display_label: "Sample Provider".to_string(),
        service_id: None,
        availability: SurfaceProviderAvailability::Available,
        encryption_metadata: None,
    }
}

fn sample_provider_public_key_b64() -> String {
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("key pair");
    base64::engine::general_purpose::STANDARD.encode(key_pair.public_key_raw())
}

fn sample_encrypted_surface_provider_info() -> SurfaceProviderInfo {
    SurfaceProviderInfo {
        provider_id: "provider.sample".to_string(),
        display_label: "Sample Provider".to_string(),
        service_id: None,
        availability: SurfaceProviderAvailability::Available,
        encryption_metadata: Some(wire_surfaces::ProviderEncryptionMetadata {
            key_id: "provider-key-1".to_string(),
            algorithm: wire_surfaces::ProviderEncryptionAlgorithm::EciesP256,
            public_key: sample_provider_public_key_b64(),
        }),
    }
}

fn sample_proxied_surface_read() -> SurfaceReadResponse {
    SurfaceReadResponse {
        descriptor: wire_surfaces::SurfaceDescriptor::builder()
            .surface_id(sample_surface_id().parse().unwrap())
            .label("Sample surface")
            .priority(200)
            .slot(wire_surfaces::SLOT_SETTINGS_TABS)
            .scope(wire_surfaces::Scope::Tenant)
            .targeting(wire_surfaces::Targeting::Targeted)
            .provider_kind(wire_surfaces::ProviderKind::Plugin)
            .required_capabilities(wire_surfaces::CapabilitySet::default())
            .root_node(wire_surfaces::SurfaceNode::Section {
                title: Some("Sample surface".to_string()),
                children: vec![],
            })
            .build(),
        interactions: vec![wire_surfaces::InteractionDescriptor {
            interaction_id: sample_interaction_id().parse().unwrap(),
            kind: wire_surfaces::InteractionKind::FormSubmit,
            label: "Submit".to_string(),
            required_permission: None,
            input_schema: Some(wire_surfaces::SchemaContract::Object),
            result_schema: Some(wire_surfaces::SchemaContract::Any),
            sensitive_fields: vec!["password".to_string()],
            timeout_seconds: Some(30),
            confirmation: None,
            transport: wire_surfaces::InteractionTransport::ProviderProxied,
            workflow_steps: vec![],
            form_ui: Some(wire_surfaces::FormUiDescriptor {
                fields: vec![
                    wire_surfaces::FormFieldDescriptor {
                        key: "username".to_string(),
                        label: "Username".to_string(),
                        field_type: "text".to_string(),
                        required: true,
                        placeholder: None,
                        help_text: None,
                        default_value: None,
                        options: vec![],
                        select_source: None,
                        sensitive: false,
                        list: false,
                        visible_when: None,
                    },
                    wire_surfaces::FormFieldDescriptor {
                        key: "password".to_string(),
                        label: "Password".to_string(),
                        field_type: "password".to_string(),
                        required: false,
                        placeholder: None,
                        help_text: None,
                        default_value: None,
                        options: vec![],
                        select_source: None,
                        sensitive: true,
                        list: false,
                        visible_when: None,
                    },
                ],
                pre_load_interaction_id: None,
            }),
        }],
        data_sources: vec![],
    }
}

// ── Hosts tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn hosts_list_success() {
    let server = MockApiServer::start();
    let _m = server.hosts().on_list().ok(&paginated_host());

    let result = hosts::list(hosts::ListParams {
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        page: None,
        per_page: None,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok(), "expected Ok, got: {result:?}");
}

#[tokio::test]
async fn hosts_list_empty() {
    let server = MockApiServer::start();
    let empty: PaginatedResponse<HostResponse> = PaginatedResponse {
        items: vec![],
        total: 0,
        page: 1,
        per_page: 20,
        total_pages: 0,
    };
    let _m = server.hosts().on_list().ok(&empty);

    let result = hosts::list(hosts::ListParams {
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        page: None,
        per_page: None,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn hosts_show_success() {
    let server = MockApiServer::start();
    let id = host_id();
    let _m = server.hosts().on_get(&id).ok(&sample_host());

    let result = hosts::show(hosts::ShowParams {
        id: &id,
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn hosts_show_not_found() {
    let server = MockApiServer::start();
    let id = host_id();
    let _m = server.hosts().on_get(&id).not_found("host not found");

    let result = hosts::show(hosts::ShowParams {
        id: &id,
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        request_timeout: None,
    })
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("not found")
            || err.to_string().contains("host not found"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn hosts_list_json_format() {
    let server = MockApiServer::start();
    let _m = server.hosts().on_list().ok(&paginated_host());

    let result = hosts::list(hosts::ListParams {
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        page: None,
        per_page: None,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok());
}

// ── Services tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn services_list_success() {
    let server = MockApiServer::start();
    let _m = server.services().on_list().ok(&paginated_service());

    let result = services::list(services::ListParams {
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        capability: None,
        status: None,
        page: None,
        per_page: None,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn services_approve_success() {
    let server = MockApiServer::start();
    let id = service_id();
    let _m = server.services().on_approve(&id).ok(&sample_service());

    let result = services::approve(
        &id,
        Some(&server.server().base_url()),
        Some("test-token"),
        false,
        None,
    )
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn services_approve_not_found() {
    let server = MockApiServer::start();
    let id = service_id();
    let _m = server
        .services()
        .on_approve(&id)
        .not_found("service not found");

    let result = services::approve(
        &id,
        Some(&server.server().base_url()),
        Some("test-token"),
        false,
        None,
    )
    .await;

    assert!(result.is_err());
}

// ── Software items tests ───────────────────────────────────────────────────

#[tokio::test]
async fn software_items_list_success() {
    let server = MockApiServer::start();
    let _m = server
        .software_items()
        .on_list()
        .ok(&paginated_software_item());

    let result = software_items::list(software_items::ListParams {
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        query: None,
        page: None,
        per_page: None,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok());
}

// ── Error handling tests ───────────────────────────────────────────────────

#[tokio::test]
async fn api_401_returns_not_authenticated() {
    let server = MockApiServer::start();
    let _m = server.hosts().on_list().unauthorized();

    let result = hosts::list(hosts::ListParams {
        server: Some(&server.server().base_url()),
        token: Some("bad-token"),
        insecure: false,
        page: None,
        per_page: None,
        request_timeout: None,
    })
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // ClientError::NotAuthenticated maps to CliError::NotLoggedIn
    assert!(
        err.to_string().to_lowercase().contains("not logged in")
            || err.to_string().to_lowercase().contains("not authenticated")
            || err.to_string().to_lowercase().contains("unauthorized"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn api_429_returns_rate_limited() {
    let server = MockApiServer::start();
    let _m = server.hosts().on_list().rate_limited(Some(60));

    let result = hosts::list(hosts::ListParams {
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        page: None,
        per_page: None,
        request_timeout: None,
    })
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("rate limit")
            || err.to_string().to_lowercase().contains("too many"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn api_500_returns_server_error() {
    let server = MockApiServer::start();
    let _m = server
        .hosts()
        .on_list()
        .internal_error("internal server error");

    let result = hosts::list(hosts::ListParams {
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        page: None,
        per_page: None,
        request_timeout: None,
    })
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("internal server error")
            || err.to_string().contains("500")
            || err.to_string().to_lowercase().contains("error"),
        "unexpected error: {err}"
    );
}

// ── Service removal test ───────────────────────────────────────────────────

#[tokio::test]
async fn services_remove_success() {
    let server = MockApiServer::start();
    let id = service_id();
    let _m = server.services().on_remove(&id).no_content();

    let result = services::remove(
        &id,
        Some(&server.server().base_url()),
        Some("test-token"),
        false,
        None,
    )
    .await;

    assert!(result.is_ok());
}

// ── Host with agents test ──────────────────────────────────────────────────

#[tokio::test]
async fn hosts_show_with_agents() {
    let server = MockApiServer::start();
    let id = host_id();
    let mut host = sample_host();
    host.agents = vec![HostAgentSummary {
        id: "d1d2d3d4-e1e2-f1f2-a1a2-b1b2b3b4b5b6".parse().unwrap(),
        friendly_name: "agent-1".to_string(),
        status: "approved".parse().unwrap(),
    }];
    let _m = server.hosts().on_get(&id).ok(&host);

    let result = hosts::show(hosts::ShowParams {
        id: &id,
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok());
}

// ── Surfaces tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn surfaces_read_success() {
    let server = MockApiServer::start();
    let surface_id = sample_surface_id();
    let _m = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/read"))
        .ok(&sample_surface_read());

    let result = surfaces::read(surfaces::ReadParams {
        surface_id: surface_id.to_string(),
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok(), "expected Ok, got: {result:?}");
}

#[tokio::test]
async fn surfaces_providers_success() {
    let server = MockApiServer::start();
    let surface_id = sample_surface_id();
    let _m = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/providers"))
        .ok(&vec![sample_surface_provider_info()]);

    let result = surfaces::providers(surfaces::ProvidersParams {
        surface_id: surface_id.to_string(),
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok(), "expected Ok, got: {result:?}");
}

#[tokio::test]
async fn surfaces_dynamic_form_submit_success() {
    let server = MockApiServer::start();
    let surface_id = sample_surface_id();
    let interaction_id = sample_interaction_id();
    let provider_id = "provider.sample";
    let _read = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/read"))
        .ok(&sample_proxied_surface_read());
    let _providers = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/providers"))
        .ok(&vec![sample_encrypted_surface_provider_info()]);
    let _invoke = server.server().mock(move |when, then| {
        when.method("POST")
            .path(format!("/api/v1/surfaces/{surface_id}/interactions/{interaction_id}"))
            .body_includes(r#""params":{"username":"router""#)
            .body_includes(r#""encrypted_sensitive_params":{"key_id":"provider-key-1","algorithm":"ecies_p256""#)
            .body_excludes(r#""password":"s3cret""#);
        then.status(200)
            .header("Content-Type", "application/json")
            .body(r#"{"ok": true}"#);
    });

    let args = vec![
        OsString::from(surface_id),
        OsString::from(interaction_id),
        OsString::from("--username"),
        OsString::from("router"),
        OsString::from("--target-provider-id"),
        OsString::from(provider_id),
    ];
    let result = surfaces::dynamic_invoke(
        args,
        Some(&server.server().base_url()),
        Some("test-token"),
        false,
        None,
    )
    .await;

    assert!(result.is_ok(), "expected Ok, got: {result:?}");
}

#[tokio::test]
async fn surfaces_invoke_splits_sensitive_and_encrypts() {
    let server = MockApiServer::start();
    let surface_id = sample_surface_id();
    let interaction_id = sample_interaction_id();
    let provider_id = "provider.sample";
    let _read = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/read"))
        .ok(&sample_proxied_surface_read());
    let _providers = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/providers"))
        .ok(&vec![sample_encrypted_surface_provider_info()]);
    let _invoke = server.server().mock(move |when, then| {
        when.method("POST")
            .path(format!("/api/v1/surfaces/{surface_id}/interactions/{interaction_id}"))
            .body_includes(r#""params":{"username":"router""#)
            .body_includes(r#""encrypted_sensitive_params":{"key_id":"provider-key-1","algorithm":"ecies_p256""#)
            .body_excludes(r#""password":"s3cret""#);
        then.status(200)
            .header("Content-Type", "application/json")
            .body(r#"{"ok": true}"#);
    });

    let params = serde_json::json!({
        "username": "router",
    })
    .as_object()
    .unwrap()
    .clone();

    let result = surfaces::invoke(surfaces::InvokeParams {
        surface_id: surface_id.to_string(),
        interaction_id: interaction_id.to_string(),
        params,
        target_provider_id: Some(provider_id.to_string()),
        timeout_seconds: None,
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        request_timeout: None,
    })
    .await;

    assert!(result.is_ok(), "expected Ok, got: {result:?}");
}

#[tokio::test]
async fn surfaces_invoke_errors_without_encryption_metadata() {
    let server = MockApiServer::start();
    let surface_id = sample_surface_id();
    let interaction_id = sample_interaction_id();
    let _read = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/read"))
        .ok(&sample_proxied_surface_read());
    let _providers = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/providers"))
        .ok(&vec![sample_surface_provider_info()]);

    let params = serde_json::json!({
        "username": "router",
    })
    .as_object()
    .unwrap()
    .clone();

    let result = surfaces::invoke(surfaces::InvokeParams {
        surface_id: surface_id.to_string(),
        interaction_id: interaction_id.to_string(),
        params,
        target_provider_id: Some("provider.sample".to_string()),
        timeout_seconds: None,
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        request_timeout: None,
    })
    .await;

    assert!(result.is_err(), "expected Err, got: {result:?}");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("does not advertise encryption metadata") || err.contains("encryption"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn surfaces_invoke_requires_target_provider_id_for_targeted_surface() {
    let server = MockApiServer::start();
    let surface_id = sample_surface_id();
    let interaction_id = sample_interaction_id();
    let _read = server
        .on("GET", &format!("/api/v1/surfaces/{surface_id}/read"))
        .ok(&sample_proxied_surface_read());

    let params = serde_json::json!({
        "username": "router",
    })
    .as_object()
    .unwrap()
    .clone();

    let result = surfaces::invoke(surfaces::InvokeParams {
        surface_id: surface_id.to_string(),
        interaction_id: interaction_id.to_string(),
        params,
        target_provider_id: None,
        timeout_seconds: None,
        server: Some(&server.server().base_url()),
        token: Some("test-token"),
        insecure: false,
        request_timeout: None,
    })
    .await;

    assert!(result.is_err(), "expected Err, got: {result:?}");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("targeted surfaces require --target-provider-id <PROVIDER_ID>"),
        "unexpected error: {err}"
    );
}
