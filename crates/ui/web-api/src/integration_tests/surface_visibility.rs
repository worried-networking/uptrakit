//! Route-level effective-enablement matrix over the surfaces legs
//! (spec 2026-07-27 §3.5 / §5.3), driven by the synthetic Instance-scoped
//! fixture plugin — deliberately NOT gated on `dashboard-icons`.

use uptrakit_plugin_infrastructure_core::testing::instance_surface_fixture as fixture;

use crate::test_harness::fixtures::register_admin_and_tenant_user;
use crate::test_harness::{TestApp, synthetic_instance_catalog};

async fn matrix_app(boot_enabled: bool) -> (TestApp, String, String) {
    let app = TestApp::with_plugin_surfaces(Some(synthetic_instance_catalog(boot_enabled))).await;
    let (admin_token, tenant_token) = register_admin_and_tenant_user(&app).await;
    (app, admin_token, tenant_token)
}

async fn set_live_enabled(app: &TestApp, admin_token: &str, enabled: bool) {
    let status = app
        .client()
        .put_json(
            &format!("/api/v1/instance-plugins/{}/enabled", fixture::TYPE_ID),
            &serde_json::json!({ "enabled": enabled }),
        )
        .bearer(admin_token)
        .send_status()
        .await;
    assert_eq!(
        status,
        http::StatusCode::OK,
        "instance-plugin toggle must succeed"
    );
}

async fn listed_surface_ids(app: &TestApp, token: &str) -> Vec<String> {
    let (status, body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/surfaces")
        .bearer(token)
        .send_json()
        .await;
    assert_eq!(
        status,
        http::StatusCode::OK,
        "list_surfaces must return 200"
    );
    // SurfaceResponse #[serde(flatten)]s the descriptor, so surface_id is a
    // TOP-LEVEL key (web-api-types/src/surfaces.rs:22) — indexing a nested
    // "descriptor" object would read Null and make every negative membership
    // assertion vacuously green.
    body.iter()
        .filter_map(|s| s["surface_id"].as_str().map(str::to_string))
        .collect()
}

/// Admission canary: the fixture's wire registration must pass the REAL
/// admission pipeline (`bootstrap_plugin`, not the admission-bypassing stub
/// path) — otherwise every matrix/provider-origin test dies as an
/// unattributable setup panic inside the harness.
#[tokio::test]
async fn fixture_registration_passes_surface_admission() {
    let registry = crate::surface_registry::SurfaceRegistry::new(
        crate::surface_registry::SurfaceRegistryConfig::default(),
    );
    let mut registrations = synthetic_instance_catalog(true).surface_registrations();
    let registration = registrations
        .pop()
        .expect("fixture catalog yields one registration");
    assert!(
        registry.bootstrap_plugin(registration).is_ok(),
        "fixture registration must survive surface admission"
    );
}

/// Boot-disabled fixture must be absent from `/api/v1/surfaces` even at the
/// raw-byte level — asserted via `send_bytes` (not `send_json`) so the check
/// is independent of any particular deserialization shape and proves the
/// filter runs before the response is ever serialized to JSON.
#[tokio::test]
async fn boot_disabled_fixture_is_absent_from_surfaces_list_raw_bytes() {
    let (app, _admin_token, tenant_token) = matrix_app(false).await;

    let (status, body) = app
        .client()
        .get("/api/v1/surfaces")
        .bearer(&tenant_token)
        .send_bytes()
        .await;

    assert_eq!(
        status,
        http::StatusCode::OK,
        "list_surfaces must return 200"
    );
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        !body_str.contains(fixture::SURFACE_ID),
        "boot-disabled fixture surface id must not appear anywhere in the raw response body"
    );
}

/// Fetches `(status, body)` for a leg against the given surface id.
async fn leg_response(
    app: &TestApp,
    token: &str,
    uri: &str,
) -> (http::StatusCode, axum::body::Bytes) {
    app.client().get(uri).bearer(token).send_bytes().await
}

/// Fetches `(status, body)` for the POST invoke leg against the given surface id.
async fn leg_response_post(
    app: &TestApp,
    token: &str,
    uri: &str,
) -> (http::StatusCode, axum::body::Bytes) {
    app.client()
        .post_json(uri, &serde_json::json!({ "params": {} }))
        .bearer(token)
        .send_bytes()
        .await
}

/// Asserts a leg's response for the fixture surface is byte-identical to the
/// same leg's response for an unknown surface (spec §5.3: no existence
/// side-channel — never the distinct NoTenantCompatibleProvider message).
async fn assert_leg_matches_unknown(app: &TestApp, token: &str, leg: &str) {
    let real = leg_response(app, token, &leg.replace("{sid}", fixture::SURFACE_ID)).await;
    let unknown = leg_response(app, token, &leg.replace("{sid}", "no.such-surface")).await;
    assert_eq!(real.0, http::StatusCode::NOT_FOUND, "leg {leg} must 404");
    assert_eq!(
        real, unknown,
        "leg {leg}: hidden-surface response must be byte-identical to unknown-surface"
    );
}

/// POST-invoke variant of [`assert_leg_matches_unknown`]: asserts the invoke
/// leg for the fixture surface is byte-identical to the same leg for an
/// unknown surface (spec §5.3: no existence side-channel).
async fn assert_invoke_leg_matches_unknown(app: &TestApp, token: &str) {
    let real = leg_response_post(
        app,
        token,
        &format!("/api/v1/surfaces/{}/interactions/ping", fixture::SURFACE_ID),
    )
    .await;
    let unknown = leg_response_post(
        app,
        token,
        "/api/v1/surfaces/no.such-surface/interactions/ping",
    )
    .await;
    assert_eq!(real.0, http::StatusCode::NOT_FOUND, "invoke leg must 404");
    assert_eq!(
        real, unknown,
        "invoke leg: hidden-surface response must be byte-identical to unknown-surface"
    );
}

/// Row 2 (§3.5): boot-enabled, live-disabled — absent + 404 immediately, no
/// restart, for BOTH tiers; admin summary shows the disable took effect
/// while running_enabled (boot state) stays true.
#[tokio::test]
async fn live_disable_takes_effect_immediately_on_every_leg_for_every_tier() {
    let (app, admin_token, tenant_token) = matrix_app(true).await;
    set_live_enabled(&app, &admin_token, true).await;
    assert!(
        listed_surface_ids(&app, &tenant_token)
            .await
            .contains(&fixture::SURFACE_ID.to_string())
    );

    // Flip through the production write path (the instance-plugins route),
    // never by poking the ArcSwap directly.
    set_live_enabled(&app, &admin_token, false).await;

    for token in [&tenant_token, &admin_token] {
        assert!(
            !listed_surface_ids(&app, token)
                .await
                .contains(&fixture::SURFACE_ID.to_string()),
            "live-disabled surface must vanish from the list without restart"
        );
        assert_leg_matches_unknown(&app, token, "/api/v1/surfaces/{sid}/providers").await;
        assert_leg_matches_unknown(&app, token, "/api/v1/surfaces/{sid}").await;
        assert_invoke_leg_matches_unknown(&app, token).await;
    }

    let (status, body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/instance-plugins")
        .bearer(&admin_token)
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::OK);
    let row = body
        .iter()
        .find(|p| p["plugin_type"] == fixture::TYPE_ID)
        .expect("fixture must appear in the admin instance-plugins list");
    assert_eq!(row["enabled"], false, "live disable must be reflected");
    assert_eq!(
        row["running_enabled"], true,
        "running_enabled intentionally reflects boot state (pending-restart badge input)"
    );
}

/// Row 3 (§3.5): boot-disabled, live-enabled (pending restart) — absent + 404
/// for both tiers; admin summary shows enabled=true / running_enabled=false.
#[tokio::test]
async fn pending_restart_enabled_surface_absent_for_every_tier() {
    let (app, admin_token, tenant_token) = matrix_app(false).await;
    set_live_enabled(&app, &admin_token, true).await;

    for token in [&tenant_token, &admin_token] {
        assert!(
            !listed_surface_ids(&app, token)
                .await
                .contains(&fixture::SURFACE_ID.to_string())
        );
        assert_leg_matches_unknown(&app, token, "/api/v1/surfaces/{sid}/providers").await;
        assert_leg_matches_unknown(&app, token, "/api/v1/surfaces/{sid}").await;
        assert_invoke_leg_matches_unknown(&app, token).await;
    }

    let (_, body): (_, Vec<serde_json::Value>) = app
        .client()
        .get("/api/v1/instance-plugins")
        .bearer(&admin_token)
        .send_json()
        .await;
    let row = body
        .iter()
        .find(|p| p["plugin_type"] == fixture::TYPE_ID)
        .expect("fixture row");
    assert_eq!(row["enabled"], true);
    assert_eq!(row["running_enabled"], false, "pending-restart state");
}

/// Row 4 (§3.5): boot-disabled, no live row (absent = disabled, D4) — absent
/// + 404 for both tiers.
#[tokio::test]
async fn boot_disabled_without_row_is_absent_for_every_tier() {
    let (app, admin_token, tenant_token) = matrix_app(false).await;
    for token in [&tenant_token, &admin_token] {
        assert!(
            !listed_surface_ids(&app, token)
                .await
                .contains(&fixture::SURFACE_ID.to_string())
        );
        assert_leg_matches_unknown(&app, token, "/api/v1/surfaces/{sid}/providers").await;
        assert_leg_matches_unknown(&app, token, "/api/v1/surfaces/{sid}").await;
        assert_invoke_leg_matches_unknown(&app, token).await;
    }
}

/// Row 1 positive complements the smoke test: read + providers 200 for both
/// tiers when effectively enabled.
#[tokio::test]
async fn effectively_enabled_surface_readable_for_every_tier() {
    let (app, admin_token, tenant_token) = matrix_app(true).await;
    set_live_enabled(&app, &admin_token, true).await;
    for token in [&tenant_token, &admin_token] {
        let (status, _) = leg_response(
            &app,
            token,
            &format!("/api/v1/surfaces/{}", fixture::SURFACE_ID),
        )
        .await;
        assert_eq!(
            status,
            http::StatusCode::OK,
            "read leg must serve the surface"
        );
        let (status, _) = leg_response(
            &app,
            token,
            &format!("/api/v1/surfaces/{}/providers", fixture::SURFACE_ID),
        )
        .await;
        assert_eq!(
            status,
            http::StatusCode::OK,
            "providers leg must serve the surface"
        );
    }
}

/// Smoke: boot-enabled + live-enabled fixture is listed and dispatchable.
#[tokio::test]
async fn boot_and_live_enabled_fixture_is_listed_and_dispatchable() {
    let (app, admin_token, tenant_token) = matrix_app(true).await;
    set_live_enabled(&app, &admin_token, true).await;

    assert!(
        listed_surface_ids(&app, &tenant_token)
            .await
            .contains(&fixture::SURFACE_ID.to_string()),
        "effectively-enabled fixture surface must be listed for tenant users"
    );

    let (status, body): (_, serde_json::Value) = app
        .client()
        .post_json(
            &format!(
                "/api/v1/surfaces/{}/interactions/{}",
                fixture::SURFACE_ID,
                fixture::INTERACTION_ID
            ),
            &serde_json::json!({ "params": {} }),
        )
        .bearer(&tenant_token)
        .send_json()
        .await;
    assert_eq!(status, http::StatusCode::OK, "invoke must dispatch: {body}");
    assert_eq!(body["pong"], true, "fixture handler result must round-trip");
}
