#![expect(
    clippy::indexing_slicing,
    reason = "integration test code: panics are acceptable in test helpers (db_test! macro means functions are not annotated #[test])"
)]

use crate::database_helpers::fixtures::{insert_service, register_and_get_token};
use crate::database_helpers::harness::TestHarness;
use crate::database_helpers::macros::db_test;
use uptrakit_shared_db::entity::service::ServiceStatus;

async fn test_batch_deactivate_services(harness: &TestHarness) {
    let client = harness.client();
    let token = register_and_get_token(&client).await;

    let svc1 = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;
    let svc2 = insert_service(&harness.db, harness.tenant_id, ServiceStatus::Approved).await;

    let (status, body): (_, serde_json::Value) = client
        .post_json(
            "/api/v1/services/batch",
            &serde_json::json!({
                "action": "deactivate",
                "ids": [svc1.id, svc2.id]
            }),
        )
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(
        body["succeeded"].as_array().is_some(),
        "response should contain succeeded array"
    );

    // Verify services are deactivated.
    let (_, list_body): (_, serde_json::Value) = client
        .get("/api/v1/services")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(list_body["total"], 0, "all services should be deactivated");
}

db_test!(batch_deactivate_services, test_batch_deactivate_services);
