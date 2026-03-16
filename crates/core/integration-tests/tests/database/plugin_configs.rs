use crate::database_helpers::fixtures::register_and_get_token;
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_list_plugin_types(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/plugin-types")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(body.as_array().is_some());
    assert!(
        !body.as_array().expect("array").is_empty(),
        "at least one plugin type should be registered"
    );
}

db_test!(list_plugin_types, test_list_plugin_types);

async fn test_create_plugin_config(harness: &TestHarness) {
    let client = harness.client();
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

db_test!(create_plugin_config, test_create_plugin_config);

async fn test_delete_plugin_config(harness: &TestHarness) {
    let client = harness.client();
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

db_test!(delete_plugin_config, test_delete_plugin_config);
