use crate::database_helpers::fixtures::{register_and_get_token, register_user};
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_list_users(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) =
        client.get("/api/v1/users").bearer(&token).send_json().await;

    assert_eq!(status, http::StatusCode::OK);
    // Users list returns a plain JSON array.
    let items = body.as_array().expect("users array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["email"], "owner@test.local");
}

db_test!(list_users, test_list_users);

async fn test_update_user_roles(harness: &TestHarness) {
    let client = harness.client();

    let (_, first) = register_user(&client, "owner@test.local", "StrongPassword1!").await;
    let owner_token = first.access_token.expose_secret();

    // Re-open registration.
    client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(owner_token)
        .send_status()
        .await;

    let (_, second) = register_user(&client, "user2@test.local", "StrongPassword2!").await;
    let user2_id = second.user.id;

    // Get available roles.
    let (_, roles_body): (_, serde_json::Value) = client
        .get("/api/v1/roles")
        .bearer(owner_token)
        .send_json()
        .await;
    let roles = roles_body.as_array().expect("roles array");
    assert!(!roles.is_empty());

    let role_id = roles[0]["id"].as_str().expect("role id");

    let status = client
        .put_json(
            &format!("/api/v1/users/{user2_id}/roles"),
            &serde_json::json!({ "role_ids": [role_id] }),
        )
        .bearer(owner_token)
        .send_status()
        .await;

    assert_eq!(status, http::StatusCode::OK);
}

db_test!(update_user_roles, test_update_user_roles);
