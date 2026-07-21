//! Smoke test for the stub surface provider harness (Task 7). Confirms the
//! `TestApp::with_stub_surfaces` scaffolding dispatches a `POST` interaction
//! invocation through the real router, stamping `InteractionHttpMethod::Post`
//! on the recorded `SurfaceActionRequest` — the parity Plan 3's GET/PUT/DELETE
//! router tests depend on.

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use crate::test_harness::{StubInteraction, StubSurfaceCalls};
use uptrakit_wire::surfaces::{InteractionHttpMethod, InteractionKind};

#[tokio::test]
async fn stub_provider_records_post_invoke_through_router() {
    let (app, calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "save",
            kind: InteractionKind::MutationAction,
            http_method: None,
            params: vec![],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let response = client
        .post_json(
            "/api/v1/surfaces/test.stub/interactions/save",
            &serde_json::json!({ "params": { "name": "x" } }),
        )
        .bearer(&token)
        .send()
        .await;

    assert_eq!(response.status(), http::StatusCode::OK);
    let recorded = calls.lock();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].method, InteractionHttpMethod::Post);
    assert_eq!(
        recorded[0].params.get("name"),
        Some(&serde_json::json!("x"))
    );
}

/// Exercises the two arbitrary-method `TestClient` builders this task adds
/// (`head()` / `request()`) against an existing GET-registered route, so
/// Plan 3's future GET/PUT/DELETE surface router tests have a proven-working
/// starting point.
#[tokio::test]
async fn arbitrary_method_test_client_helpers_reach_router() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let head_status = client
        .head("/api/v1/surfaces")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(head_status, http::StatusCode::OK);

    let request_status = client
        .request(http::Method::GET, "/api/v1/surfaces")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(request_status, http::StatusCode::OK);
}
