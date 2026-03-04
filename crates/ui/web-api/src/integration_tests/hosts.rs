use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    insert_host, insert_service, link_service_host, register_and_get_token,
};
use uptrakit_shared_db::entity::service::ServiceStatus;

#[tokio::test]
async fn list_hosts_empty_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/hosts")
        .bearer(&token)
        .send_json()
        .await;

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

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/hosts")
        .bearer(&token)
        .send_json()
        .await;

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
