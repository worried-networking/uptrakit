#![expect(
    clippy::indexing_slicing,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use crate::database_helpers::fixtures::register_and_get_token;
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_get_registration_settings(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .get("/api/v1/settings/registration")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(body["mode"].as_str().is_some());
}

db_test!(get_registration_settings, test_get_registration_settings);

async fn test_update_registration_settings_persists(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (_, serde_json::Value) = client
        .put_json(
            "/api/v1/settings/registration",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["mode"], "open");

    // Verify persistence by reading again.
    let (s2, body2): (_, serde_json::Value) = client
        .get("/api/v1/settings/registration")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(s2, http::StatusCode::OK);
    assert_eq!(body2["mode"], "open");
}

db_test!(
    update_registration_settings_persists,
    test_update_registration_settings_persists
);
