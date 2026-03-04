use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;

#[tokio::test]
async fn list_plugin_types_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    // The response is an array of plugin type metadata.
    assert!(body.as_array().is_some());
    assert!(
        !body.as_array().expect("array").is_empty(),
        "at least one plugin type should be registered"
    );
}

#[tokio::test]
async fn create_config_returns_201() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "My GitHub Config",
                "plugin_type": "releases_github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(body["id"].as_str().is_some());
    assert_eq!(body["name"], "My GitHub Config");
}

#[tokio::test]
async fn delete_config_returns_204() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (_, created): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/plugin-configs",
            &serde_json::json!({
                "name": "To Delete Config",
                "plugin_type": "releases_github",
                "config": {}
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    let id = created["id"].as_str().expect("id");

    let status = client
        .delete(&format!("/api/v1/plugin-configs/{id}"))
        .bearer(&token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::NO_CONTENT);
}
