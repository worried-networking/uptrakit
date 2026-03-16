use crate::database_helpers::fixtures::{
    insert_host, insert_service, link_service_host, register_and_get_token,
};
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;
use uptrakit_shared_db::entity::service::ServiceStatus;

async fn test_list_hosts_empty(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) =
        client.get("/api/v1/hosts").bearer(&token).send_json().await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("data array").len(), 0);
}

db_test!(list_hosts_empty, test_list_hosts_empty);

async fn test_list_hosts_returns_linked(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;
    let host = insert_host(&harness.db, harness.tenant_id).await;
    link_service_host(&harness.db, svc.id, host.id).await;

    let (status, body): (_, serde_json::Value) =
        client.get("/api/v1/hosts").bearer(&token).send_json().await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["items"].as_array().expect("data array").len(), 1);
}

db_test!(list_hosts_returns_linked, test_list_hosts_returns_linked);

async fn test_get_host_detail(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;
    let host = insert_host(&harness.db, harness.tenant_id).await;
    link_service_host(&harness.db, svc.id, host.id).await;

    let (status, body): (_, serde_json::Value) = client
        .get(&format!("/api/v1/hosts/{}", host.id))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["id"], host.id.to_string());
}

db_test!(get_host_detail, test_get_host_detail);

async fn test_deactivate_host(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;
    let host = insert_host(&harness.db, harness.tenant_id).await;
    link_service_host(&harness.db, svc.id, host.id).await;

    let status = client
        .delete(&format!("/api/v1/hosts/{}", host.id))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

db_test!(deactivate_host, test_deactivate_host);
