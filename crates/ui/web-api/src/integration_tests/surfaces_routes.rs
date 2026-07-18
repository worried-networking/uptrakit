//! Route-level tests for the surface endpoints: resource-shaped read-model
//! path (spec 2026-07-16, decision A1) and `Cache-Control: private, no-store`
//! on every surface GET response.
#![expect(
    clippy::unwrap_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::register_and_get_token;
use uptrakit_wire::surfaces;
use uuid::Uuid;

/// Minimal targeted surface registration with NO required_permission, so the
/// assertions are independent of the registering user's permission set.
fn test_surface_registration(tenant_id: Uuid) -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: "provider-a".to_string(),
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
                .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                .label("SSH Guest Panel")
                .priority(100)
                .slot(surfaces::SLOT_SOFTWARE_TABS)
                .scope(surfaces::Scope::Tenant)
                .targeting(surfaces::Targeting::Targeted)
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

fn register_test_surface(app: &TestApp) {
    app.state
        .surface_proxy_deps
        .registry
        .register_provider_for_test(
            test_surface_registration(app.tenant_id),
            Some(Uuid::now_v7()),
            Some("uptrakit-agent-ssh"),
        );
}

fn assert_private_no_store(resp: &http::Response<axum::body::Body>) {
    assert_eq!(
        resp.headers()
            .get(http::header::CACHE_CONTROL)
            .expect("Cache-Control header present")
            .to_str()
            .expect("header is ascii"),
        "private, no-store"
    );
}

#[tokio::test]
async fn old_read_path_is_gone() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    register_test_surface(&app);

    let status = client
        .get("/api/v1/surfaces/ssh.guest.panel/read")
        .bearer(&token)
        .send_status()
        .await;
    assert_eq!(status, http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn read_model_served_at_resource_path_with_no_store() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    register_test_surface(&app);

    let resp = client
        .get("/api/v1/surfaces/ssh.guest.panel")
        .bearer(&token)
        .send()
        .await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    assert_private_no_store(&resp);

    let bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .expect("body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(body["descriptor"]["surface_id"], "ssh.guest.panel");
    assert!(body["interactions"].is_array());
    assert!(body["data_sources"].is_array());
}

#[tokio::test]
async fn list_surfaces_sets_private_no_store() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    register_test_surface(&app);

    let resp = client.get("/api/v1/surfaces").bearer(&token).send().await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    assert_private_no_store(&resp);
}

#[tokio::test]
async fn list_surface_providers_sets_private_no_store() {
    let app = TestApp::new().await;
    let client = app.client();
    let token = register_and_get_token(&client).await;
    register_test_surface(&app);

    let resp = client
        .get("/api/v1/surfaces/ssh.guest.panel/providers")
        .bearer(&token)
        .send()
        .await;
    assert_eq!(resp.status(), http::StatusCode::OK);
    assert_private_no_store(&resp);
}
