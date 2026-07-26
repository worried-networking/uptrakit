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

/// Fail-first (Task 3, Step 0): anti-probe ordering. Registers the same
/// `interaction_id` under two methods that do NOT include `GET`, one of
/// which (`PUT`) is gated by a permission string that maps to
/// `Permission::Other` (never granted to any role). A `GET` request against
/// this interaction has no registered handler for that method, so the
/// registry resolves `MethodNotAllowed`. The security property under test:
/// the missing-permission check on candidate methods MUST run before the
/// 405/Allow disclosure, so an unauthorized caller gets 403 — never a 405
/// that would leak which methods exist.
#[tokio::test]
async fn missing_permission_is_403_before_method_disclosure() {
    let (app, calls): (TestApp, StubSurfaceCalls) = TestApp::with_stub_surfaces(vec![
        StubInteraction {
            interaction_id: "gated",
            kind: InteractionKind::MutationAction,
            http_method: Some(InteractionHttpMethod::Put),
            params: vec![],
            required_permission: Some("__nonexistent_test_permission__".to_string()),
        },
        StubInteraction {
            interaction_id: "gated",
            kind: InteractionKind::MutationAction,
            http_method: Some(InteractionHttpMethod::Delete),
            params: vec![],
            required_permission: None,
        },
    ])
    .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let response = client
        .request(
            http::Method::GET,
            "/api/v1/surfaces/test.stub/interactions/gated",
        )
        .bearer(&token)
        .send()
        .await;

    assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
    assert!(
        response.headers().get(http::header::ALLOW).is_none(),
        "a 403 anti-probe response must never disclose the Allow set"
    );
    assert!(calls.lock().is_empty(), "provider must never be reached");
}

/// Fail-first (Task 3, Step 0): HEAD must be auto-derived from the GET
/// registration and short-circuited before it ever reaches the provider.
/// Asserts the full triple: 200 status (proves utoipa-axum's route
/// composition actually derives HEAD from the GET registration — a
/// router-level 405 here means the routing needs an explicit HEAD route,
/// not a handler-level workaround), an empty recorded-calls list (proves the
/// provider was never invoked), and the mandatory
/// `Cache-Control: private, no-store` header on every surface GET response.
#[tokio::test]
async fn head_does_not_reach_provider() {
    let (app, calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "peek",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let response = client
        .head("/api/v1/surfaces/test.stub/interactions/peek")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(response.status(), http::StatusCode::OK);
    assert!(
        calls.lock().is_empty(),
        "HEAD must never reach the provider"
    );
    assert_eq!(
        response
            .headers()
            .get(http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, no-store")
    );
}
