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
