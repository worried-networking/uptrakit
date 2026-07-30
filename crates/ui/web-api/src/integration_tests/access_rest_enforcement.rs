#![expect(
    clippy::expect_used,
    reason = "test helper functions are not covered by allow-expect-in-tests"
)]

//! D-row REST enforcement tests for the M1.4a reference family (hosts).
//!
//! Fixture rule (spec §Tests, MANDATORY): authority mutations go through
//! **direct DB writes + explicit `invalidate_subjects`**, never the
//! role-update endpoint (it invalidates nothing until M1.6a — a warmed
//! cache would vacuously green these tests). Direct `Entity::` access
//! against `&app.db` here is a deliberate, acknowledged deviation from the
//! "use `TenantDb` for tenant-scoped queries" rule: fixture staging
//! intentionally bypasses the governed paths (mirroring the engine's own
//! test module and `access_grants.rs` tests) — production code must never
//! copy this.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uptrakit_shared_db::access_grants::{GrantSubject, NewGrant, delete_grant, insert_grant};
use uptrakit_shared_db::entity::{role, user_role};
use uptrakit_shared_types::access::{ActionPattern, Selector};
use uptrakit_web_api_types::api_tokens::{CreateApiTokenRequest, CreateApiTokenResponse};
use uptrakit_web_api_types::error::ErrorResponse;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{register_and_get_token, register_user};
use crate::test_harness::http_client::TestClient;

/// Register a second (non-owner) user, strip its auto-assigned viewer role,
/// and flush the engine cache. Returns (user_id, access_token).
///
/// The user id comes straight from the registration response
/// (`AuthResponse.user: UserResponse { id: Uuid, .. }`) — never query the
/// `users` table by email here: `user::Model.email` is `MaskedEmail`, not
/// `String`, so a bare string `.eq()` filter is a typed-column trap.
async fn staged_zero_grant_user(app: &TestApp) -> (uuid::Uuid, String) {
    let client = app.client();
    // Owner must exist first so the new user is a non-first registration.
    let owner_token = register_and_get_token(&client).await;
    // First-user setup closes registration; re-open it so a second user can
    // sign up (mirrors `fixtures::register_admin_and_tenant_user`).
    let reopen = client
        .put_json(
            "/api/v1/settings/access",
            &serde_json::json!({ "mode": "open" }),
        )
        .bearer(&owner_token)
        .header("if-match", "W/\"settings-v0\"")
        .send_status()
        .await;
    assert_eq!(
        reopen,
        http::StatusCode::OK,
        "failed to re-open registration"
    );
    let (status, auth) = register_user(&client, "zero-grant@test.local", "TestPassword123!").await;
    assert_eq!(status, http::StatusCode::CREATED);
    let user_id = auth.user.id;
    user_role::Entity::delete_many()
        .filter(user_role::Column::UserId.eq(user_id))
        .exec(&app.db)
        .await
        .expect("strip auto-assigned roles");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);
    (user_id, auth.access_token.expose_secret().to_string())
}

/// Mint a `upk_`-prefixed API token for a user authenticated via `jwt_bearer`
/// (the create-api-token route is authenticated-but-ungoverned — no
/// permission/action extractor gates it, so a zero-grant user can mint one)
/// and return the raw token string.
async fn mint_api_token(client: &TestClient, jwt_bearer: &str, name: &str) -> String {
    let req = CreateApiTokenRequest {
        name: name.to_string(),
    };
    let (status, resp): (http::StatusCode, CreateApiTokenResponse) = client
        .post_json("/api/v1/auth/api-tokens", &req)
        .bearer(jwt_bearer)
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::CREATED, "api token mint failed");
    resp.token.expose_secret().to_string()
}

#[tokio::test]
async fn d2_no_credential_is_401() {
    let app = TestApp::new().await;
    let status = app.client().get("/api/v1/hosts").send_status().await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn d1_owner_jwt_reads_hosts() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    let status = client
        .get("/api/v1/hosts")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::OK);
}

#[tokio::test]
async fn d1_owner_api_token_reads_hosts() {
    let app = TestApp::new().await;
    let client = app.client();
    let jwt = register_and_get_token(&client).await;
    let upk_token = mint_api_token(&client, &jwt, "owner-cli").await;
    let status = client
        .get("/api/v1/hosts")
        .bearer(&upk_token)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::OK);
}

#[tokio::test]
async fn d3_zero_grant_user_is_403_with_generic_body() {
    let app = TestApp::new().await;
    let (_user_id, token) = staged_zero_grant_user(&app).await;
    let (status, body) = app
        .client()
        .get("/api/v1/hosts")
        .bearer(&token)
        .send_bytes()
        .await;
    assert_eq!(status, http::StatusCode::FORBIDDEN);
    let parsed: ErrorResponse = serde_json::from_slice(&body).expect("json error body");
    assert_eq!(parsed.error, "Insufficient permissions");
    assert_eq!(parsed.code, None);
    // Whole-body equality check: the fixed message + `code: null` is the
    // *entire* body — no grant/selector/reason detail is ever embedded.
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(
        value,
        serde_json::json!({ "error": "Insufficient permissions", "code": null })
    );
}

#[tokio::test]
async fn d3_zero_grant_api_token_is_403_with_generic_body() {
    let app = TestApp::new().await;
    let (_user_id, jwt) = staged_zero_grant_user(&app).await;
    let client = app.client();
    let upk_token = mint_api_token(&client, &jwt, "zero-grant-cli").await;
    let (status, body) = client
        .get("/api/v1/hosts")
        .bearer(&upk_token)
        .send_bytes()
        .await;
    assert_eq!(status, http::StatusCode::FORBIDDEN);
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(
        value,
        serde_json::json!({ "error": "Insufficient permissions", "code": null })
    );
}

#[tokio::test]
async fn immediate_effect_grant_insert_then_delete() {
    let app = TestApp::new().await;
    let (user_id, token) = staged_zero_grant_user(&app).await;
    let client = app.client();

    assert_eq!(
        client
            .get("/api/v1/hosts")
            .bearer(&token)
            .send_status()
            .await,
        http::StatusCode::FORBIDDEN
    );

    let patterns = vec!["hosts:read".parse::<ActionPattern>().expect("pattern")];
    let grant_id = insert_grant(
        &app.db,
        NewGrant {
            subject: GrantSubject::User(user_id),
            tenant_id: Some(app.tenant_id),
            patterns: &patterns,
            selector: Selector::All,
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("insert grant");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);
    assert_eq!(
        client
            .get("/api/v1/hosts")
            .bearer(&token)
            .send_status()
            .await,
        http::StatusCode::OK,
        "grant must take effect on the next request, no re-login"
    );

    delete_grant(&app.db, grant_id).await.expect("delete grant");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);
    assert_eq!(
        client
            .get("/api/v1/hosts")
            .bearer(&token)
            .send_status()
            .await,
        http::StatusCode::FORBIDDEN,
        "revocation must take effect on the next request"
    );
}

#[tokio::test]
async fn pairing_rule_operator_only_reads_hosts() {
    // Pins the INTENDED hosts:read pairing widening (spec §6): operator-only
    // principals could NOT list hosts under the legacy claim model.
    let app = TestApp::new().await;
    let (user_id, token) = staged_zero_grant_user(&app).await;
    // Assign ONLY the operator role, directly in the DB (the role endpoint
    // invalidates nothing until M1.6a).
    let operator_role_id = role::Entity::find()
        .filter(role::Column::Name.eq("operator"))
        .one(&app.db)
        .await
        .expect("query roles")
        .expect("seeded operator role")
        .id;
    user_role::Entity::insert(user_role::ActiveModel {
        tenant_id: sea_orm::Set(app.tenant_id),
        user_id: sea_orm::Set(user_id),
        role_id: sea_orm::Set(operator_role_id),
        assigned_at: sea_orm::Set(time::OffsetDateTime::now_utc()),
    })
    .exec(&app.db)
    .await
    .expect("assign operator role");
    app.state.access_engine.invalidate_subjects(&[user_id], &[]);

    assert_eq!(
        app.client()
            .get("/api/v1/hosts")
            .bearer(&token)
            .send_status()
            .await,
        http::StatusCode::OK
    );
}
