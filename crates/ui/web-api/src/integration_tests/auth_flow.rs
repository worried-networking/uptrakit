use http::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{
    login_user, open_registration, refresh_token, register_user, stage_user_with_grant,
    stage_zero_role_user,
};
use uptrakit_shared_types::access::Action;
use uptrakit_wire::surfaces;

/// Test-local copy of `access_catalog.rs`'s `registration_for_test_stub`
/// helper (not exported — each integration-test module that needs a
/// `surface.test.stub` registration builds its own minimal valid one; ledger
/// #17: `TestApp::with_stub_surfaces` cannot be used here, its swapped
/// registry is invisible to the engine the app was wired with).
#[expect(
    clippy::expect_used,
    reason = "invalid literal surface id/action here would be a test-helper bug, not a runtime failure"
)]
fn registration_for_test_stub(provider_id: &str, tenant_id: Uuid) -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: provider_id.to_string(),
            provider_kind: surfaces::ProviderKind::Service,
            provider_namespace: "service".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::TargetedTargeting,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Tenant,
            tenant_id: Some(tenant_id.to_string()),
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor::builder()
                .surface_id(surfaces::SurfaceId::new("test.stub").expect("valid surface id"))
                .label("Test Stub")
                .priority(100)
                .slot("software.tabs")
                .scope(surfaces::Scope::Tenant)
                .targeting(surfaces::Targeting::Targeted)
                .required_action(
                    "surface.test.stub:use"
                        .parse::<Action>()
                        .expect("valid action"),
                )
                .provider_kind(surfaces::ProviderKind::Service)
                .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::TargetedTargeting,
                ]))
                .root_node(surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                })
                .build(),
            interactions: vec![],
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

#[tokio::test]
async fn register_first_user_returns_201() {
    let app = TestApp::new().await;
    let client = app.client();

    let (status, auth) = register_user(&client, "owner@test.local", "StrongPassword1!").await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert!(!auth.access_token.expose_secret().is_empty());
    assert!(!auth.refresh_token.expose_secret().is_empty());
    assert_eq!(auth.token_type, "Bearer");
    assert_eq!(auth.user.email, "owner@test.local");
    // First user gets the "owner" role — all actions should be present.
    assert!(
        !auth.user.actions.is_empty(),
        "first user should have actions"
    );
}

#[tokio::test]
async fn register_second_user_gets_user_role() {
    let app = TestApp::new().await;
    let client = app.client();

    // First user (gets owner role).
    let (s1, first) = register_user(&client, "owner@test.local", "StrongPassword1!").await;
    assert_eq!(s1, http::StatusCode::CREATED);

    // Re-open registration (initial setup closes it after first user).
    let reopen_status = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(first.access_token.expose_secret())
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;
    assert_eq!(reopen_status, http::StatusCode::OK);

    // Second user (gets user role — fewer actions).
    let (s2, second) = register_user(&client, "user2@test.local", "StrongPassword2!").await;
    assert_eq!(s2, http::StatusCode::CREATED);
    assert!(
        second.user.actions.len() < first.user.actions.len(),
        "second user should have fewer actions than owner"
    );
}

#[tokio::test]
async fn register_duplicate_email_returns_409() {
    let app = TestApp::new().await;
    let client = app.client();

    let (s1, first) = register_user(&client, "dup@test.local", "StrongPassword1!").await;
    assert_eq!(s1, http::StatusCode::CREATED);

    // Re-open registration (initial setup closes it after first user).
    client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(first.access_token.expose_secret())
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;

    let status = client
        .post_json(
            "/api/v1/auth/register",
            &serde_json::json!({
                "email": "dup@test.local",
                "first_name": "Dup",
                "last_name": "User",
                "password": "StrongPassword1!"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::CONFLICT);
}

#[tokio::test]
async fn register_invalid_email_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .post_json(
            "/api/v1/auth/register",
            &serde_json::json!({
                "email": "",
                "first_name": "A",
                "last_name": "B",
                "password": "StrongPassword1!"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn register_short_password_returns_400() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .post_json(
            "/api/v1/auth/register",
            &serde_json::json!({
                "email": "short@test.local",
                "first_name": "A",
                "last_name": "B",
                "password": "abc"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn login_valid_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();

    register_user(&client, "login@test.local", "StrongPassword1!").await;
    let (status, auth) = login_user(&client, "login@test.local", "StrongPassword1!").await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(!auth.access_token.expose_secret().is_empty());
    assert_eq!(auth.user.email, "login@test.local");
}

#[tokio::test]
async fn login_wrong_password_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    register_user(&client, "wrong@test.local", "StrongPassword1!").await;
    let status = client
        .post_json(
            "/api/v1/auth/login",
            &serde_json::json!({
                "email": "wrong@test.local",
                "password": "WrongPassword!!"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_nonexistent_user_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .post_json(
            "/api/v1/auth/login",
            &serde_json::json!({
                "email": "ghost@test.local",
                "password": "StrongPassword1!"
            }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_valid_token_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();

    let (_, auth) = register_user(&client, "refresh@test.local", "StrongPassword1!").await;
    let (status, refreshed) = refresh_token(&client, auth.refresh_token.expose_secret()).await;

    assert_eq!(status, http::StatusCode::OK);
    assert!(!refreshed.access_token.expose_secret().is_empty());
    assert!(!refreshed.refresh_token.expose_secret().is_empty());
}

#[tokio::test]
async fn refresh_invalid_token_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client
        .post_json(
            "/api/v1/auth/refresh",
            &serde_json::json!({ "refresh_token": "totally-invalid-token" }),
        )
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_rotates_token() {
    let app = TestApp::new().await;
    let client = app.client();

    let (_, auth) = register_user(&client, "rotate@test.local", "StrongPassword1!").await;
    let old_refresh = auth.refresh_token.expose_secret().to_string();

    // Use the refresh token once — should succeed.
    let (s1, new_tokens) = refresh_token(&client, &old_refresh).await;
    assert_eq!(s1, http::StatusCode::OK);

    // Using the old refresh token again should fail (it was rotated).
    let s2 = client
        .post_json(
            "/api/v1/auth/refresh",
            &serde_json::json!({ "refresh_token": old_refresh }),
        )
        .send_status()
        .await;
    assert_eq!(s2, http::StatusCode::UNAUTHORIZED);

    // The new refresh token should still work.
    let (s3, _) = refresh_token(&client, new_tokens.refresh_token.expose_secret()).await;
    assert_eq!(s3, http::StatusCode::OK);
}

#[tokio::test]
async fn me_with_valid_jwt_returns_200() {
    let app = TestApp::new().await;
    let client = app.client();

    let (_, auth) = register_user(&client, "me@test.local", "StrongPassword1!").await;
    let (status, user): (_, serde_json::Value) = client
        .get("/api/v1/auth/me")
        .bearer(auth.access_token.expose_secret())
        .send_json()
        .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(user["email"], "me@test.local");
}

#[tokio::test]
async fn me_without_auth_returns_401() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client.get("/api/v1/auth/me").send_status().await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_revokes_tokens() {
    let app = TestApp::new().await;
    let client = app.client();

    let (_, auth) = register_user(&client, "logout@test.local", "StrongPassword1!").await;
    let access = auth.access_token.expose_secret();
    let refresh = auth.refresh_token.expose_secret().to_string();

    // Logout.
    let status = client
        .post_json(
            "/api/v1/auth/logout",
            &serde_json::json!({ "refresh_token": refresh }),
        )
        .bearer(access)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::NO_CONTENT);

    // Refresh should now fail.
    let s2 = client
        .post_json(
            "/api/v1/auth/refresh",
            &serde_json::json!({ "refresh_token": refresh }),
        )
        .send_status()
        .await;
    assert_eq!(s2, http::StatusCode::UNAUTHORIZED);
}

/// D13: a wildcard grant (`software:*`) must expand to the concrete catalog
/// verbs on `me`, scoped to its own resource — not leak into others.
#[tokio::test]
async fn me_expands_wildcard_grant_to_concrete_actions() {
    let app = TestApp::new().await;
    let client = app.client();
    open_registration(&app).await;
    let (_user_id, token) = stage_user_with_grant(
        &app,
        "wildcard-me@example.com",
        &["software:*"],
        Some(app.tenant_id),
    )
    .await;

    let (status, body): (StatusCode, Value) = client
        .get("/api/v1/auth/me")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["authority"], "ok");
    let actions: Vec<String> = body["actions"]
        .as_array()
        .expect("actions array")
        .iter()
        .map(|v| v.as_str().expect("action string").to_string())
        .collect();
    assert!(
        actions.contains(&"software:read".to_string()),
        "{actions:?}"
    );
    assert!(
        actions.contains(&"software:update".to_string()),
        "{actions:?}"
    );
    assert!(
        actions.iter().all(|a| a.starts_with("software:")),
        "wildcard must not leak beyond its resource: {actions:?}"
    );
}

/// D13: a principal with zero grants still gets a healthy (`ok`) authority
/// and an empty action list — distinct from the engine-failure leg covered
/// by `routes/auth.rs`'s `me_engine_unavailable_is_200_with_unavailable_authority`.
#[tokio::test]
async fn me_zero_grant_principal_gets_empty_actions_with_ok_authority() {
    let app = TestApp::new().await;
    let client = app.client();
    let (_user_id, token) = stage_zero_role_user(&app).await;

    let (status, body): (StatusCode, Value) = client
        .get("/api/v1/auth/me")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["authority"], "ok");
    assert_eq!(body["actions"].as_array().expect("actions array").len(), 0);
}

/// D13: the login response embeds the same actions/authority shape as `me`.
#[tokio::test]
async fn login_response_carries_actions_and_authority() {
    let app = TestApp::new().await;
    let client = app.client();
    open_registration(&app).await;
    let (_user_id, _token) = stage_user_with_grant(
        &app,
        "login-actions@example.com",
        &["hosts:read"],
        Some(app.tenant_id),
    )
    .await;

    let (status, body): (StatusCode, Value) = client
        .post_json(
            "/api/v1/auth/login",
            &serde_json::json!({
                "email": "login-actions@example.com",
                "password": "TestPassword123!"
            }),
        )
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["authority"], "ok");
    let actions = body["user"]["actions"].as_array().expect("actions array");
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(actions[0], "hosts:read");
}

/// D13: the `surface.*` dynamic-action section of `me` tracks live registry
/// state both ways — appears once a matching grant + registration exist,
/// disappears on unregistration. `dynamic_actions()` reads the registry per
/// request (not cached), so no explicit cache invalidation is needed here
/// (mirrors `dynamic_actions_appear_and_disappear_with_registry_state` in
/// `access_catalog.rs`, which proves the same for the catalog endpoint).
#[tokio::test]
async fn me_includes_registered_dynamic_surface_action_when_granted() {
    let app = TestApp::new().await;
    let client = app.client();
    open_registration(&app).await;
    let (_user_id, token) = stage_user_with_grant(
        &app,
        "dynamic-me@example.com",
        &["surface.test.stub:use"],
        Some(app.tenant_id),
    )
    .await;
    let registry = &app.state.surface_proxy_deps.registry;
    let service_id = Uuid::now_v7();
    registry
        .register_service(
            service_id,
            "test-stub",
            Some(app.tenant_id),
            registration_for_test_stub("service.test-stub", app.tenant_id),
        )
        .expect("valid registration must admit");

    let (status, body): (StatusCode, Value) = client
        .get("/api/v1/auth/me")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);
    let actions: Vec<String> = body["actions"]
        .as_array()
        .expect("actions array")
        .iter()
        .map(|v| v.as_str().expect("action string").to_string())
        .collect();
    assert!(
        actions.contains(&"surface.test.stub:use".to_string()),
        "{actions:?}"
    );

    registry.unregister_service(&service_id);
    let (_, body): (StatusCode, Value) = client
        .get("/api/v1/auth/me")
        .bearer(&token)
        .send_json()
        .await;
    let actions = body["actions"].as_array().expect("actions array");
    assert!(
        actions.is_empty(),
        "deregistered action must disappear: {actions:?}"
    );
}
