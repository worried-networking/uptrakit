//! Router-level test battery for the method-mapped surface REST routes
//! (Plan 3). Every test drives the REAL router via `TestClient` — never a
//! handler function directly — so the assertions cover route composition
//! (method dispatch, item-segment overlay, 404/405/403/422 shaping,
//! `Cache-Control`, audit emission) exactly as an external HTTP caller would
//! observe it. None of these tests gate on Proxmox or any other plugin being
//! present; they run entirely against the `"test.stub"` provider from
//! [`crate::test_harness::TestApp::with_stub_surfaces`].
#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use crate::test_harness::{StubInteraction, StubSurfaceCalls};
use uptrakit_web_api_types::error::ErrorResponse;
use uptrakit_wire::surfaces::{
    InteractionHttpMethod, InteractionKind, ParamFieldDescriptor, SchemaContract,
};

/// Decodes a raw router response body as the standard [`ErrorResponse`]
/// envelope. Mirrors the `error_body` idiom in `routes/surfaces.rs`'s test
/// module, adapted to the `http::Response<Body>` shape `TestClient::send`
/// returns (rather than `axum::response::Response` from a direct handler
/// call).
async fn error_body(response: http::Response<axum::body::Body>) -> ErrorResponse {
    use http_body_util::BodyExt;
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body should read")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("response body should deserialize as ErrorResponse")
}

/// Polls for the most recent audit row for `action_type`, retrying briefly
/// since audit `Event` emission (`emit_event`) is async/fire-and-forget.
/// Copied verbatim from `tenant_audit_row_for_action` in
/// `routes/surfaces.rs`'s test module (the canonical denied-audit idiom this
/// task's brief asks to reuse).
async fn audit_row_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> uptrakit_shared_db::entity::audit_log::Model {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    for _ in 0..50 {
        if let Some(row) = uptrakit_shared_db::entity::audit_log::Entity::find()
            .filter(uptrakit_shared_db::entity::audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(uptrakit_shared_db::entity::audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("expected audit row for action {action_type}");
}

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

/// Spec test 1 + 7 (positive leg): the three-tier GET query coercion —
/// reserved (`page`), declared (`count: Integer`), and undeclared
/// passthrough (`note`) — all land in the dispatched `SurfaceActionRequest`
/// with the correctly-typed *values*, not merely the right count of params.
/// Also folds in spec test 14 (`implicit_provider_resolution_shape`): this
/// is the request the brief designates to prove the caller sends NO
/// `target_provider_id` anywhere in the request (production callers on this
/// route never set one) while the proxy still resolves and dispatches to
/// the sole registered provider implicitly — and that the resolved id is
/// carried only via the dedicated field, never leaked into `params`.
#[tokio::test]
async fn get_dataload_coerces_reserved_declared_and_passthrough() {
    let (app, calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "items",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![ParamFieldDescriptor::new("count", SchemaContract::Integer)],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let response = client
        .get("/api/v1/surfaces/test.stub/interactions/items?page=2&count=7&note=plain")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(response.status(), http::StatusCode::OK);
    let recorded = calls.lock();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].params.get("page"), Some(&serde_json::json!(2)));
    assert_eq!(recorded[0].params.get("count"), Some(&serde_json::json!(7)));
    assert_eq!(
        recorded[0].params.get("note"),
        Some(&serde_json::json!("plain"))
    );
    // Spec test 14: the GET request above carries no `target_provider_id`
    // anywhere (no such query key was sent), yet the proxy still resolves
    // one implicitly since exactly one provider is registered for this
    // surface — and it arrives only via the dedicated field, never leaked
    // into `params`.
    assert_eq!(
        recorded[0].target_provider_id.as_deref(),
        Some("test.stub.provider")
    );
    assert!(!recorded[0].params.contains_key("target_provider_id"));
}

/// Spec test 7 (negative leg): a declared `Integer` param that fails to
/// parse is a `422` tagged `schema_validation_failed`, and the provider is
/// never reached.
#[tokio::test]
async fn get_page_abc_is_422_schema_validation_failed() {
    let (app, calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "items",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let (status, body): (http::StatusCode, ErrorResponse) = client
        .get("/api/v1/surfaces/test.stub/interactions/items?page=abc")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body.code.as_deref(), Some("schema_validation_failed"));
    assert!(calls.lock().is_empty());
}

/// Spec test 2 (direction A): POSTing a GET-only interaction id is a `405`
/// carrying `Allow: GET` and the JSON `method_not_allowed` envelope — never
/// axum's bare 405.
#[tokio::test]
async fn post_on_dataload_only_id_is_405_allow_get() {
    let (app, calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "get_only",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let response = client
        .post_json(
            "/api/v1/surfaces/test.stub/interactions/get_only",
            &serde_json::json!({}),
        )
        .bearer(&token)
        .send()
        .await;

    assert_eq!(response.status(), http::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response
            .headers()
            .get(http::header::ALLOW)
            .and_then(|v| v.to_str().ok()),
        Some("GET")
    );
    let body = error_body(response).await;
    assert_eq!(body.code.as_deref(), Some("method_not_allowed"));
    assert!(calls.lock().is_empty());
}

/// Spec test 2 (direction B): the mirror image of the previous test — GETing
/// a POST-only interaction id is a `405` carrying `Allow: POST`.
#[tokio::test]
async fn get_on_post_only_id_is_405_allow_post() {
    let (app, calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "post_only",
            kind: InteractionKind::MutationAction,
            http_method: Some(InteractionHttpMethod::Post),
            params: vec![],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let response = client
        .get("/api/v1/surfaces/test.stub/interactions/post_only")
        .bearer(&token)
        .send()
        .await;

    assert_eq!(response.status(), http::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response
            .headers()
            .get(http::header::ALLOW)
            .and_then(|v| v.to_str().ok()),
        Some("POST")
    );
    let body = error_body(response).await;
    assert_eq!(body.code.as_deref(), Some("method_not_allowed"));
    assert!(calls.lock().is_empty());
}

/// Spec test 3: the same interaction id registered under two different
/// methods dispatches to genuinely distinct handlers — proven by leaked
/// *values* (not just call counts) differing per registration, plus the
/// recorded `SurfaceActionRequest.method` matching the HTTP verb used.
#[tokio::test]
async fn same_id_two_methods_dispatch_to_distinct_handlers() {
    let (app, calls): (TestApp, StubSurfaceCalls) = TestApp::with_stub_surfaces(vec![
        StubInteraction {
            interaction_id: "items",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![],
            required_permission: None,
        },
        StubInteraction {
            interaction_id: "items",
            kind: InteractionKind::MutationAction,
            http_method: Some(InteractionHttpMethod::Post),
            params: vec![],
            required_permission: None,
        },
    ])
    .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let get_status = client
        .get("/api/v1/surfaces/test.stub/interactions/items?tag=via-get")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(get_status, http::StatusCode::OK);

    let post_status = client
        .post_json(
            "/api/v1/surfaces/test.stub/interactions/items",
            &serde_json::json!({ "params": { "tag": "via-post" } }),
        )
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(post_status, http::StatusCode::OK);

    let recorded = calls.lock();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].method, InteractionHttpMethod::Get);
    assert_eq!(recorded[1].method, InteractionHttpMethod::Post);
    assert_eq!(
        recorded[0].params.get("tag"),
        Some(&serde_json::json!("via-get"))
    );
    assert_eq!(
        recorded[1].params.get("tag"),
        Some(&serde_json::json!("via-post"))
    );
    assert_ne!(recorded[0].params.get("tag"), recorded[1].params.get("tag"));
}

/// Spec test 4: a `PUT` against an item-segment path injects the reserved
/// `id` param (overwriting anything a caller could otherwise supply), while
/// the base route (no item segment) never introduces one.
#[tokio::test]
async fn put_item_segment_delivers_reserved_id_param() {
    let (app, calls): (TestApp, StubSurfaceCalls) = TestApp::with_stub_surfaces(vec![
        StubInteraction {
            interaction_id: "items",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![],
            required_permission: None,
        },
        StubInteraction {
            interaction_id: "replace",
            kind: InteractionKind::MutationAction,
            http_method: Some(InteractionHttpMethod::Put),
            params: vec![],
            required_permission: None,
        },
    ])
    .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let put_status = client
        .put_json(
            "/api/v1/surfaces/test.stub/interactions/replace/abc-123",
            &serde_json::json!({}),
        )
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(put_status, http::StatusCode::OK);

    let get_status = client
        .get("/api/v1/surfaces/test.stub/interactions/items")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(get_status, http::StatusCode::OK);

    let recorded = calls.lock();
    assert_eq!(recorded.len(), 2);
    assert_eq!(
        recorded[0].params.get("id"),
        Some(&serde_json::json!("abc-123"))
    );
    assert!(
        !recorded[1].params.contains_key("id"),
        "base route without an item segment must not inject an `id` param"
    );
}

/// Spec test 5: an interaction id with no registration at all is a `404`
/// regardless of which HTTP method is used to reach it. `POST`/`PUT` use
/// the JSON convenience wrappers (rather than a bare `request()` call)
/// because `invoke_surface_interaction`/`update_surface_interaction` extract
/// a body — an empty, content-type-less request would fail the extractor
/// before the interaction lookup ever ran, masking the 404 this test targets.
#[tokio::test]
async fn unknown_interaction_is_404_on_every_method() {
    let (app, _calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "known",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let uri = "/api/v1/surfaces/test.stub/interactions/does-not-exist";

    let get_status = client.get(uri).bearer(&token).send_status().await;
    assert_eq!(get_status, http::StatusCode::NOT_FOUND);

    let post_status = client
        .post_json(uri, &serde_json::json!({}))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(post_status, http::StatusCode::NOT_FOUND);

    let put_status = client
        .put_json(uri, &serde_json::json!({}))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(put_status, http::StatusCode::NOT_FOUND);

    let delete_status = client.delete(uri).bearer(&token).send_status().await;
    assert_eq!(delete_status, http::StatusCode::NOT_FOUND);
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
///
/// Extended in Task 4 (spec test 6, case (b)): `POST` is a third method with
/// no registration at all for `"gated"`, so it exercises the same anti-probe
/// branch from a pure method-mismatch angle, and asserts the permission-denied
/// audit row records the *requested* method (`post`), not one of the
/// registered candidates.
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

    let post_response = client
        .post_json(
            "/api/v1/surfaces/test.stub/interactions/gated",
            &serde_json::json!({}),
        )
        .bearer(&token)
        .send()
        .await;

    assert_eq!(post_response.status(), http::StatusCode::FORBIDDEN);
    assert!(
        post_response.headers().get(http::header::ALLOW).is_none(),
        "a 403 anti-probe response must never disclose the Allow set"
    );
    assert!(calls.lock().is_empty(), "provider must never be reached");

    // The anti-probe branch must audit like the matched-permission path: a
    // permission-denied row recording the *requested* method (`post`), not
    // one of the candidate methods actually registered for `gated`.
    let row = audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::SURFACE_ACTION_INVOKE,
    )
    .await;
    assert_eq!(
        row.outcome,
        uptrakit_audit_log::AuditOutcome::Denied.as_str()
    );
    let details = row
        .details_json
        .as_ref()
        .expect("permission denial audit should include details");
    assert_eq!(details["http_method"], "post");
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

/// Spec test 9: `Cache-Control: private, no-store` is present on every
/// surface GET response, success or error — asserted here for both a `200`
/// and the `422` schema-validation failure from
/// `get_page_abc_is_422_schema_validation_failed`.
#[tokio::test]
async fn get_responses_carry_private_no_store() {
    let (app, _calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "items",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let ok_response = client
        .get("/api/v1/surfaces/test.stub/interactions/items")
        .bearer(&token)
        .send()
        .await;
    assert_eq!(ok_response.status(), http::StatusCode::OK);
    assert_eq!(
        ok_response
            .headers()
            .get(http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("private, no-store")
    );

    let invalid_response = client
        .get("/api/v1/surfaces/test.stub/interactions/items?page=abc")
        .bearer(&token)
        .send()
        .await;
    assert_eq!(
        invalid_response.status(),
        http::StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        invalid_response
            .headers()
            .get(http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("private, no-store")
    );
}

/// A `POST` to an item-segment path is never valid — the router hardcodes
/// `Allow: GET, PUT, DELETE` for that shape regardless of what's registered
/// — and, like every other `405`, it must be the JSON `ErrorResponse`
/// envelope, never axum's bare 405.
#[tokio::test]
async fn post_item_segment_is_enveloped_405() {
    let (app, calls): (TestApp, StubSurfaceCalls) =
        TestApp::with_stub_surfaces(vec![StubInteraction {
            interaction_id: "items",
            kind: InteractionKind::DataLoad,
            http_method: Some(InteractionHttpMethod::Get),
            params: vec![],
            required_permission: None,
        }])
        .await;
    let client = app.client();
    let token = register_and_get_token(&client).await;

    let response = client
        .post_json(
            "/api/v1/surfaces/test.stub/interactions/items/abc",
            &serde_json::json!({}),
        )
        .bearer(&token)
        .send()
        .await;

    assert_eq!(response.status(), http::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response
            .headers()
            .get(http::header::ALLOW)
            .and_then(|v| v.to_str().ok()),
        Some("GET, PUT, DELETE")
    );
    let body = error_body(response).await;
    assert_eq!(body.code.as_deref(), Some("method_not_allowed"));
    assert!(calls.lock().is_empty());
}
