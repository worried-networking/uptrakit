//! Integration tests for the canonical-host pending-OIDC-flow purge.
//!
//! Unlike `oidc_purge.rs`, this module is **not** gated behind the `oidc`
//! feature: `settings_oauth::update_oauth_settings`'s canonical-host arm
//! (and the bulk purge helper it calls,
//! `uptrakit_web_api_queries::queries::oidc_providers::purge_all_pending_flows_in_tx`)
//! is unconditional, because the MCP-OAuth settings route it lives in is
//! itself ungated. Coverage for that path must therefore compile and run in
//! the zero-feature build too — seeding goes straight through the
//! (ungated) `pending_oidc_flow` entity rather than through
//! `crate::auth::oidc_state::OidcFlowStore` or the (gated) OIDC-provider
//! CRUD routes, both of which are unavailable here.

#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures;
use sea_orm::{ActiveModelTrait, EntityTrait, PaginatorTrait, Set};
use uptrakit_shared_db::entity::pending_oidc_flow;

/// Seed one pending OIDC flow row for a fresh provider id, keyed by
/// `csrf_state`. Inserted directly via the entity — no real OIDC provider
/// row is required, since `pending_oidc_flows.provider_id` carries no
/// foreign-key constraint, and no encryption/AAD setup is required since
/// `EncryptedString::plaintext_for_test` stores the value as-is.
async fn seed_flow(app: &TestApp, csrf_state: &str) {
    let now = time::OffsetDateTime::now_utc();
    pending_oidc_flow::ActiveModel {
        csrf_state: Set(csrf_state.to_string()),
        provider_id: Set(uuid::Uuid::now_v7()),
        pkce_verifier: Set(uptrakit_crypto::EncryptedString::plaintext_for_test(
            "test_pkce_verifier".to_string(),
        )),
        nonce: Set("test_nonce".to_string()),
        redirect_uri: Set(String::new()),
        return_origin: Set(String::new()),
        created_at: Set(now),
        expires_at: Set(now + time::Duration::seconds(600)),
    }
    .insert(&app.db)
    .await
    .expect("seed pending OIDC flow");
}

/// Count every pending flow row in the table.
async fn total_flow_count(app: &TestApp) -> u64 {
    pending_oidc_flow::Entity::find()
        .count(&app.db)
        .await
        .expect("count all pending flows")
}

// ── canonical-host change purges every pending flow ─────────────────────────

#[tokio::test]
async fn canonical_host_change_purges_all_flows() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = fixtures::register_and_get_token(&client).await;

    seed_flow(&app, "canonical-change-state-1").await;
    seed_flow(&app, "canonical-change-state-2").await;
    assert_eq!(total_flow_count(&app).await, 2);

    let put_status = client
        .put_json(
            "/api/v1/global-settings/oauth",
            &serde_json::json!({ "canonical_host": "sso.example.com" }),
        )
        .bearer(&token)
        .header("if-match", "W/\"global-settings-v0\"")
        .send_status()
        .await;
    assert_eq!(put_status, http::StatusCode::OK);

    assert_eq!(
        total_flow_count(&app).await,
        0,
        "a canonical-host change must purge every pending OIDC flow"
    );
}

// ── canonical-host resend of the same value keeps flows intact ─────────────

#[tokio::test]
async fn canonical_host_resend_same_value_keeps_flows() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = fixtures::register_and_get_token(&client).await;

    // First PUT establishes the canonical host (this itself purges — expected,
    // it is a real change from unset to set).
    let first_status = client
        .put_json(
            "/api/v1/global-settings/oauth",
            &serde_json::json!({ "canonical_host": "sso.example.com" }),
        )
        .bearer(&token)
        .header("if-match", "W/\"global-settings-v0\"")
        .send_status()
        .await;
    assert_eq!(first_status, http::StatusCode::OK);

    // Capture the fresh ETag for the second PUT.
    let get_resp = client
        .get("/api/v1/global-settings/oauth")
        .bearer(&token)
        .send()
        .await;
    assert_eq!(get_resp.status(), http::StatusCode::OK);
    let etag = get_resp
        .headers()
        .get("etag")
        .expect("ETag header present")
        .to_str()
        .expect("ETag is ASCII")
        .to_string();

    seed_flow(&app, "canonical-resend-state").await;
    assert_eq!(total_flow_count(&app).await, 1);

    // Second PUT resends the exact same canonical_host value (this is what
    // the frontend does on every OAuth-settings save).
    let second_status = client
        .put_json(
            "/api/v1/global-settings/oauth",
            &serde_json::json!({ "canonical_host": "sso.example.com" }),
        )
        .bearer(&token)
        .header("if-match", &etag)
        .send_status()
        .await;
    assert_eq!(second_status, http::StatusCode::OK);

    assert_eq!(
        total_flow_count(&app).await,
        1,
        "resending the same canonical_host value must not purge pending flows"
    );
}
