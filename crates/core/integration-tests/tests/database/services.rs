#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use crate::database_helpers::fixtures::{insert_service, register_and_get_token};
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;
use uptrakit_shared_db::entity::service::ServiceStatus;

async fn test_list_services_empty(harness: &TestHarness) {
    let client = harness.client();
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

db_test!(list_services_empty, test_list_services_empty);

async fn test_list_services_returns_enrolled(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/services")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("items array").len(), 1);
    assert_eq!(body["total"], 1);
}

db_test!(
    list_services_returns_enrolled,
    test_list_services_returns_enrolled
);

async fn test_approve_service(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Pending).await;

    let (status, body): (_, serde_json::Value) = client
        .post_empty(&format!("/api/v1/services/{}/approve", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["status"], "approved");
}

db_test!(approve_service, test_approve_service);

async fn test_reject_service(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Pending).await;

    let (status, body): (_, serde_json::Value) = client
        .post_empty(&format!("/api/v1/services/{}/reject", svc.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["status"], "rejected");
}

db_test!(reject_service, test_reject_service);

async fn test_deactivate_service(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;

    let status = client
        .delete(&format!("/api/v1/services/{}", svc.id))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

db_test!(deactivate_service, test_deactivate_service);

async fn test_update_service_ping_interval(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;

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

db_test!(
    update_service_ping_interval,
    test_update_service_ping_interval
);

async fn test_get_nonexistent_service_returns_404(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let fake_id = uuid::Uuid::now_v7();
    let status = client
        .get(&format!("/api/v1/services/{fake_id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

db_test!(
    get_nonexistent_service_returns_404,
    test_get_nonexistent_service_returns_404
);
