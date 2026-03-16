use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;

async fn test_healthz_returns_200(harness: &TestHarness) {
    let client = harness.client();

    let status = client.get("/healthz").send_status().await;
    assert_eq!(status, http::StatusCode::OK);
}

db_test!(healthz_returns_200, test_healthz_returns_200);
