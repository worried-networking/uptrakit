use crate::routes::agents::find_or_create_host_and_link;
use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    insert_host, insert_service, link_service_host, register_and_get_token,
};
use sea_orm::{ActiveModelTrait, Set};
use uptrakit_internal_wire::HostInfo;
use uptrakit_shared_db::entity::service::{self, ServiceStatus};

#[tokio::test]
async fn list_hosts_empty_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) =
        client.get("/api/v1/hosts").bearer(&token).send_json().await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("data array").len(), 0);
}

#[tokio::test]
async fn list_hosts_returns_linked_hosts() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;
    let host = insert_host(&app.db, app.tenant_id).await;
    link_service_host(&app.db, svc.id, host.id).await;

    let (status, body): (_, serde_json::Value) =
        client.get("/api/v1/hosts").bearer(&token).send_json().await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("data array").len(), 1);
}

#[tokio::test]
async fn get_host_returns_detail() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;
    let host = insert_host(&app.db, app.tenant_id).await;
    link_service_host(&app.db, svc.id, host.id).await;

    let (status, body): (_, serde_json::Value) = client
        .get(&format!("/api/v1/hosts/{}", host.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["id"], host.id.to_string());
}

#[tokio::test]
async fn deactivate_host_returns_204() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;
    let host = insert_host(&app.db, app.tenant_id).await;
    link_service_host(&app.db, svc.id, host.id).await;

    let status = client
        .delete(&format!("/api/v1/hosts/{}", host.id))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

/// After deactivating a host, `find_or_create_host_and_link` must create a
/// fresh host record with a new ID rather than updating the deactivated one.
#[tokio::test]
async fn report_hosts_creates_new_record_after_deactivation() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;
    let host = insert_host(&app.db, app.tenant_id).await;
    link_service_host(&app.db, svc.id, host.id).await;

    // Deactivate the host via the REST API.
    let status = client
        .delete(&format!("/api/v1/hosts/{}", host.id))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::NO_CONTENT);

    // Simulate agent re-reporting the same machine_id.
    let host_info = HostInfo {
        machine_id: host.machine_id.clone(),
        hostname: Some("re-registered-host".to_string()),
        os_type: Some("linux".to_string()),
        os_version: Some("Ubuntu 24.04".to_string()),
        architecture: Some("x86_64".to_string()),
        ip_address: Some("10.0.0.99".to_string()),
        agent_host_id: None,
        features: None,
    };

    let result = find_or_create_host_and_link(
        &app.db,
        app.tenant_id,
        svc.id,
        &host_info,
        host_info.hostname.as_deref().unwrap_or("unknown"),
        host_info.ip_address.as_deref(),
    )
    .await
    .expect("find_or_create_host_and_link should succeed");

    let (new_host_id, is_new) = result.expect("should return Some for valid machine_id");

    // Must be a brand new host, not the deactivated one.
    assert!(is_new, "host must be reported as newly created");
    assert_ne!(
        new_host_id, host.id,
        "new host must have a different ID from the deactivated host"
    );

    // The new host must be visible via the API.
    let (get_status, body): (_, serde_json::Value) = client
        .get(&format!("/api/v1/hosts/{new_host_id}"))
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(get_status, http::StatusCode::OK);
    assert_eq!(body["hostname"], "re-registered-host");

    // The old deactivated host must still be invisible.
    let old_status = client
        .get(&format!("/api/v1/hosts/{}", host.id))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(old_status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn discover_host_ignores_deactivated_services() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let host = insert_host(&app.db, app.tenant_id).await;
    let stale_service = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;
    let active_service = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;

    link_service_host(&app.db, stale_service.id, host.id).await;
    link_service_host(&app.db, active_service.id, host.id).await;

    service::ActiveModel {
        id: Set(stale_service.id),
        deactivated_at: Set(Some(time::OffsetDateTime::now_utc())),
        ..stale_service.into()
    }
    .update(&app.db)
    .await
    .expect("deactivate stale service");

    let (status, body): (_, serde_json::Value) = client
        .post_empty(&format!("/api/v1/hosts/{}/discover", host.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["plugins_queued"], 1);
}
