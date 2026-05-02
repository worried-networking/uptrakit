#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use crate::database_helpers::fixtures::register_and_get_token;
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_create_software_item(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "My App" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert_eq!(body["name"], "My App");
    assert!(body["id"].as_str().is_some());
}

db_test!(create_software_item, test_create_software_item);

async fn test_create_empty_name_returns_400(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .post_json("/api/v1/software-items", &serde_json::json!({ "name": "" }))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

db_test!(
    create_empty_name_returns_400,
    test_create_empty_name_returns_400
);

async fn test_list_software_items(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Listed App" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/software-items")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    let items = body["items"].as_array().expect("data array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Listed App");
}

db_test!(list_software_items, test_list_software_items);

async fn test_get_software_item(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Detail App" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");
    let (status, body): (_, serde_json::Value) = client
        .get(&format!("/api/v1/software-items/{id}"))
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["name"], "Detail App");
}

db_test!(get_software_item, test_get_software_item);

async fn test_update_software_item_name(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Old Name" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");
    let (status, body): (_, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/software-items/{id}"),
            &serde_json::json!({ "name": "New Name" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["name"], "New Name");
}

db_test!(update_software_item_name, test_update_software_item_name);

async fn test_delete_software_item(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "To Delete" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/software-items/{id}"))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::NO_CONTENT);

    let status2 = client
        .get(&format!("/api/v1/software-items/{id}"))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(status2, http::StatusCode::NOT_FOUND);
}

db_test!(delete_software_item, test_delete_software_item);
