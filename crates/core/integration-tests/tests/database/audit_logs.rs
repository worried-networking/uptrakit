use crate::database_helpers::fixtures::register_and_get_token;
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_audit_log_list_returns_200(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let (status, _body): (_, serde_json::Value) = client
        .get("/api/v1/audit-logs")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
}

db_test!(audit_log_list_returns_200, test_audit_log_list_returns_200);
