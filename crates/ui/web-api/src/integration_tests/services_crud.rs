use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    insert_embedded_service, insert_service, register_and_get_token,
};
use uptrakit_shared_db::entity::service::ServiceStatus;

#[tokio::test]
async fn list_services_empty_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/services")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items array").len(), 0);
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn list_services_returns_enrolled_service() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Insert a service directly.
    insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/services")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items array").len(), 1);
    assert_eq!(body["total"], 1);
}

#[tokio::test]
async fn approve_service_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Pending).await;

    let (status, body): (_, serde_json::Value) = client
        .post_empty(&format!("/api/v1/services/{}/approve", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["status"], "approved");
}

#[tokio::test]
async fn reject_service_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Pending).await;

    let (status, body): (_, serde_json::Value) = client
        .post_empty(&format!("/api/v1/services/{}/reject", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["status"], "rejected");
}

#[tokio::test]
async fn deactivate_service_returns_204() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;

    let status = client
        .delete(&format!("/api/v1/services/{}", svc.id))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn update_service_friendly_name() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&app.db, app.tenant_id, ServiceStatus::Approved).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/services/{}", svc.id),
            &serde_json::json!({ "ping_interval_seconds": 30 }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["ping_interval_seconds"], 30);
}

#[tokio::test]
async fn get_nonexistent_service_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let fake_id = uuid::Uuid::now_v7();
    let status = client
        .get(&format!("/api/v1/services/{fake_id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deactivate_embedded_service_returns_409() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_embedded_service(&app.db, app.tenant_id).await;

    let (status, body): (_, serde_json::Value) = client
        .delete(&format!("/api/v1/services/{}", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
    assert!(
        body["error"].as_str().unwrap_or("").contains("Embedded"),
        "expected error about embedded services, got: {body}"
    );
}

#[tokio::test]
async fn get_embedded_service_shows_is_embedded() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_embedded_service(&app.db, app.tenant_id).await;

    let (status, body): (_, serde_json::Value) = client
        .get(&format!("/api/v1/services/{}", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["is_embedded"], true);
}

#[tokio::test]
async fn services_without_auth_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client.get("/api/v1/services").send_status().await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}
