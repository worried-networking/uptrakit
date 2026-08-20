//! Integration tests for pending-OIDC-flow purges on provider mutation and
//! canonical-host change (Task B4).
//!
//! Covers:
//! - `update_provider` / `activate_provider` purge the pending flows tied to
//!   the mutated provider (and, for `activate_provider`, the flows tied to
//!   any sibling provider it deactivates via the exclusivity loop).
//! - `update_oauth_settings`'s canonical-host arm purges *all* pending flows
//!   only when the normalized canonical host actually changes — never on a
//!   resend of the same value (the frontend resends `canonical_host` on
//!   every OAuth-settings PUT).

#![cfg(feature = "oidc")]
#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test code: panics on failure are acceptable"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures;
use openidconnect::{Nonce, PkceCodeVerifier};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use uptrakit_shared_db::entity::pending_oidc_flow;

/// Seed one pending OIDC flow row for `provider_id`, keyed by `csrf_state`.
/// The PKCE verifier / nonce / redirect snapshot values are never exercised
/// by these tests (no token exchange happens), so placeholder values are
/// fine.
async fn seed_flow(app: &TestApp, csrf_state: &str, provider_id: uuid::Uuid) {
    let result = app
        .state
        .oidc
        .oidc_flow_store
        .insert(
            csrf_state.to_string(),
            provider_id,
            &PkceCodeVerifier::new("test_pkce_verifier".to_string()),
            &Nonce::new("test_nonce".to_string()),
            crate::auth::oidc_state::FlowSnapshot {
                redirect_uri: String::new(),
                return_origin: None,
            },
        )
        .await;
    assert!(result.is_ok(), "seed pending OIDC flow: {result:?}");
}

/// Count pending flow rows referencing `provider_id`.
async fn flow_count_for_provider(app: &TestApp, provider_id: uuid::Uuid) -> u64 {
    pending_oidc_flow::Entity::find()
        .filter(pending_oidc_flow::Column::ProviderId.eq(provider_id))
        .count(&app.db)
        .await
        .expect("count pending flows for provider")
}

/// Count every pending flow row in the table.
async fn total_flow_count(app: &TestApp) -> u64 {
    pending_oidc_flow::Entity::find()
        .count(&app.db)
        .await
        .expect("count all pending flows")
}

/// Create an OIDC provider via the HTTP API and return its id.
async fn create_provider(app: &TestApp, token: &str, slug: &str) -> uuid::Uuid {
    let (status, created): (http::StatusCode, serde_json::Value) = app
        .client()
        .post_json(
            "/api/v1/settings/oidc-providers",
            &serde_json::json!({
                "name": slug,
                "slug": slug,
                "issuer_url": "https://auth.example.com",
                "client_id": "uptrakit",
                "client_secret": "initial-secret",
                "allow_private_network_issuers": false
            }),
        )
        .bearer(token)
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::CREATED, "create provider {slug}");
    created["id"]
        .as_str()
        .expect("provider id in response")
        .parse()
        .expect("provider id is a UUID")
}

// ── update_provider purges its own pending flows ────────────────────────────

#[tokio::test]
async fn provider_update_purges_its_pending_flows() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = fixtures::register_and_get_token(&client).await;

    let provider_id = create_provider(&app, &token, "update-purge-target").await;
    seed_flow(&app, "update-purge-state-1", provider_id).await;
    seed_flow(&app, "update-purge-state-2", provider_id).await;
    assert_eq!(flow_count_for_provider(&app, provider_id).await, 2);

    let (status, _body): (http::StatusCode, serde_json::Value) = client
        .put_json(
            &format!("/api/v1/settings/oidc-providers/{provider_id}"),
            &serde_json::json!({ "name": "update-purge-target-renamed" }),
        )
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::OK);

    assert_eq!(
        flow_count_for_provider(&app, provider_id).await,
        0,
        "update_provider must purge every pending flow tied to the provider"
    );
}

// ── activate_provider purges the deactivated siblings' pending flows ───────

#[tokio::test]
async fn provider_activate_purges_deactivated_siblings_flows() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = fixtures::register_and_get_token(&client).await;

    let provider_a = create_provider(&app, &token, "activate-sibling-a").await;
    let provider_b = create_provider(&app, &token, "activate-sibling-b").await;

    // Activate A first so it is the exclusivity-loop target for B's activation.
    let activate_a_status = client
        .post_empty(&format!(
            "/api/v1/settings/oidc-providers/{provider_a}/activate"
        ))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(activate_a_status, http::StatusCode::OK);

    seed_flow(&app, "sibling-a-state", provider_a).await;
    seed_flow(&app, "sibling-b-state", provider_b).await;
    assert_eq!(flow_count_for_provider(&app, provider_a).await, 1);
    assert_eq!(flow_count_for_provider(&app, provider_b).await, 1);

    // Activating B deactivates A via the exclusivity loop.
    let activate_b_status = client
        .post_empty(&format!(
            "/api/v1/settings/oidc-providers/{provider_b}/activate"
        ))
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(activate_b_status, http::StatusCode::OK);

    assert_eq!(
        flow_count_for_provider(&app, provider_a).await,
        0,
        "activating a sibling must purge the deactivated provider's pending flows"
    );
}

// ── canonical-host change purges every pending flow ─────────────────────────

#[tokio::test]
async fn canonical_host_change_purges_all_flows() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = fixtures::register_and_get_token(&client).await;

    let provider_id = create_provider(&app, &token, "canonical-change-target").await;
    seed_flow(&app, "canonical-change-state-1", provider_id).await;
    seed_flow(&app, "canonical-change-state-2", provider_id).await;
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

    let provider_id = create_provider(&app, &token, "canonical-resend-target").await;
    seed_flow(&app, "canonical-resend-state", provider_id).await;
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
