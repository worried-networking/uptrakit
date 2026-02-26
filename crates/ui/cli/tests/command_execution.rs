//! Integration tests for CLI command execution against a mock API server.
//!
//! Each test starts a [`MockApiServer`], registers a response, calls the
//! corresponding CLI command function with the mock server's URL and a dummy
//! token, and asserts the result.

use time::macros::datetime;
use uptrakit_cli::commands::{hosts, services, software_items};
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::mock::MockApiServer;
use uptrakit_openapi_client::types::hosts::{HostAgentSummary, HostResponse};
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::services::{MessageResponse, ServiceResponse};
use uptrakit_openapi_client::types::software_items::SoftwareItemResponse;

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
        ip_address: None,
        status: "approved".parse().unwrap(),
        client_version: Some("1.0.0".to_string()),
        last_seen_at: Some(datetime!(2025-01-01 00:00:00 UTC)),
        created_at: datetime!(2025-01-01 00:00:00 UTC),
        updated_at: datetime!(2025-01-01 00:00:00 UTC),
        ping_interval_seconds: None,
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
        enabled: true,
        discovery_state: None,
        last_checked_at: None,
        host_count: 2,
        latest_version: None,
        update_available: false,
        created_at: datetime!(2025-01-01 00:00:00 UTC),
        updated_at: datetime!(2025-01-01 00:00:00 UTC),
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
    let msg = MessageResponse {
        message: "Service removed.".to_string(),
    };
    let _m = server.services().on_remove(&id).ok(&msg);

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
