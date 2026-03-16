use crate::database_helpers::fixtures::{
    insert_host, insert_service, link_service_host, register_and_get_token,
};
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;
use uptrakit_shared_db::entity::service::ServiceStatus;

async fn test_create_host_tag(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/host-tags",
            &serde_json::json!({ "name": "production" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert_eq!(body["name"], "production");
}

db_test!(create_host_tag, test_create_host_tag);

async fn test_list_host_tags(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    client
        .post_json(
            "/api/v1/host-tags",
            &serde_json::json!({ "name": "staging" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/host-tags")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
}

db_test!(list_host_tags, test_list_host_tags);

async fn test_assign_tag_to_host(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;
    let host = insert_host(&harness.db, harness.tenant_id).await;
    link_service_host(&harness.db, svc.id, host.id).await;

    let (_, tag): (_, serde_json::Value) = client
        .post_json("/api/v1/host-tags", &serde_json::json!({ "name": "web" }))
        .bearer(&token)
        .send_json()
        .await;

    let tag_id = tag["id"].as_str().expect("tag id");

    let status = client
        .put_json(
            &format!("/api/v1/hosts/{}/tags", host.id),
            &serde_json::json!({ "tag_ids": [tag_id] }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::OK);
}

db_test!(assign_tag_to_host, test_assign_tag_to_host);

async fn test_delete_host_tag(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (_, tag): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/host-tags",
            &serde_json::json!({ "name": "to-delete" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let tag_id = tag["id"].as_str().expect("tag id");

    let status = client
        .delete(&format!("/api/v1/host-tags/{tag_id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

db_test!(delete_host_tag, test_delete_host_tag);
