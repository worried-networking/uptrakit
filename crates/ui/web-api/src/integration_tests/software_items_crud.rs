use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use uuid::Uuid;

#[tokio::test]
async fn create_returns_201() {
    let app = TestApp::new().await;
    let client = app.client();
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

#[tokio::test]
async fn list_returns_created() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create an item.
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

#[tokio::test]
async fn get_returns_detail() {
    let app = TestApp::new().await;
    let client = app.client();
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

#[tokio::test]
async fn update_name_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
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

#[tokio::test]
async fn delete_returns_204_then_get_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
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

#[tokio::test]
async fn create_empty_name_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let status = client
        .post_json("/api/v1/software-items", &serde_json::json!({ "name": "" }))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn trigger_update_on_nonexistent_item_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let status = client
        .post_json(
            &format!("/api/v1/software-items/{item_id}/hosts/{host_id}/update"),
            &serde_json::json!({ "to_version": "1.0.0" }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn check_versions_on_nonexistent_item_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let item_id = Uuid::now_v7();
    let status = client
        .post_empty(&format!("/api/v1/software-items/{item_id}/check-versions"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn assign_hosts_with_empty_list_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create a real item so the route reaches the validation check.
    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Assignable App" }),
        )
        .bearer(&token)
        .send_json()
        .await;
    let id = created["id"].as_str().expect("id");

    let status = client
        .post_json(
            &format!("/api/v1/software-items/{id}/hosts"),
            &serde_json::json!({ "host_assignments": [] }),
        )
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn approve_non_pending_item_returns_409() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    // Create an item — discovery_state defaults to None (not pending).
    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/software-items",
            &serde_json::json!({ "name": "Non-Pending App" }),
        )
        .bearer(&token)
        .send_json()
        .await;
    let id = created["id"].as_str().expect("id");

    let status = client
        .post_empty(&format!("/api/v1/software-items/{id}/approve"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
}
