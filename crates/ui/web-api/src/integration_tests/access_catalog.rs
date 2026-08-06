//! `GET /api/v1/access/catalog` (M1.6b): the authorization vocabulary as
//! data. Confirms the endpoint is authenticated-but-ungoverned (any
//! zero-grant principal reads it), pins the built-in/bundle/preset shape
//! against hardcoded literals (never re-derived from the source of truth
//! under test), and proves the `surface.*` dynamic-action section tracks
//! live registry state — appears on registration, disappears on
//! unregistration.
//!
//! Staging trap (mistake-ledger, restated): `register_user`/
//! `register_and_get_token` trigger owner bootstrap on the FIRST
//! registration, so a principal staged that way holds every permission and
//! a guard test built on it would pass on inherited authority rather than
//! the thing under test. The zero-grant principal below is staged via
//! [`stage_zero_role_user`], never the raw post-registration token.

#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]

use http::StatusCode;
use uuid::Uuid;

use crate::test_harness::TestApp;
use crate::test_harness::fixtures::stage_zero_role_user;
use uptrakit_shared_types::access::{Action, SelectorSupport};
use uptrakit_web_api_types::access_catalog::{AccessCatalogResponse, ScopePresetKind};
use uptrakit_wire::surfaces;

/// Test-local copy of `surface_action_registry.rs`'s test-module helper (not
/// exported — this module builds its own minimal valid registration for a
/// `surface.test.stub` provider).
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
async fn catalog_requires_authentication() {
    let app = TestApp::new().await;
    let client = app.client();

    let status = client.get("/api/v1/access/catalog").send_status().await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn zero_grant_principal_gets_catalog() {
    let app = TestApp::new().await;
    let client = app.client();
    let (_, token) = stage_zero_role_user(&app).await;

    let (status, _catalog): (StatusCode, AccessCatalogResponse) = client
        .get("/api/v1/access/catalog")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn catalog_shape_pins_builtins_bundles_and_presets() {
    let app = TestApp::new().await;
    let client = app.client();
    let (_, token) = stage_zero_role_user(&app).await;

    let (status, catalog): (StatusCode, AccessCatalogResponse) = client
        .get("/api/v1/access/catalog")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);

    // `hosts:read` — Host-scoped selector, present under resource `hosts`.
    let hosts_entry = catalog
        .resources
        .iter()
        .find(|r| r.resource == "hosts")
        .expect("hosts resource present");
    let hosts_read = hosts_entry
        .actions
        .iter()
        .find(|a| a.action == "hosts:read")
        .expect("hosts:read present");
    assert_eq!(hosts_read.selector_support, SelectorSupport::Host);

    // No `hosts:approve` — Hosts only carries Read/Update/Delete.
    assert!(
        !hosts_entry
            .actions
            .iter()
            .any(|a| a.action == "hosts:approve"),
        "hosts:approve must not exist in the catalog"
    );

    // `updates:trigger` — HostAndSoftware-scoped selector.
    let updates_entry = catalog
        .resources
        .iter()
        .find(|r| r.resource == "updates")
        .expect("updates resource present");
    let updates_trigger = updates_entry
        .actions
        .iter()
        .find(|a| a.action == "updates:trigger")
        .expect("updates:trigger present");
    assert_eq!(
        updates_trigger.selector_support,
        SelectorSupport::HostAndSoftware
    );

    // `settings.auth:manage` — no selector support.
    let settings_auth_entry = catalog
        .resources
        .iter()
        .find(|r| r.resource == "settings.auth")
        .expect("settings.auth resource present");
    let settings_auth_manage = settings_auth_entry
        .actions
        .iter()
        .find(|a| a.action == "settings.auth:manage")
        .expect("settings.auth:manage present");
    assert_eq!(settings_auth_manage.selector_support, SelectorSupport::None);

    // Set-equality supplement: every catalog action string, derived from the
    // same `CATALOG` the handler reads.
    let expected_actions: std::collections::BTreeSet<String> =
        uptrakit_shared_types::access::CATALOG
            .iter()
            .flat_map(|entry| entry.verbs.iter())
            .map(|verb| verb.action_str.to_string())
            .collect();
    let actual_actions: std::collections::BTreeSet<String> = catalog
        .resources
        .iter()
        .flat_map(|r| r.actions.iter())
        .map(|a| a.action.clone())
        .collect();
    assert_eq!(
        expected_actions, actual_actions,
        "no surfaces are registered in this test, so the response's action set must equal \
         CATALOG's exactly — a subset check would miss a spurious extra"
    );

    // Role bundles: 5 entries; `owner`'s roles pinned to a hardcoded literal
    // list (never re-derived by calling `RoleBundle::Owner.roles()`).
    assert_eq!(catalog.role_bundles.len(), 5);
    let owner_bundle = catalog
        .role_bundles
        .iter()
        .find(|b| b.name == "owner")
        .expect("owner bundle present");
    assert_eq!(
        owner_bundle.roles,
        vec![
            "viewer",
            "operator",
            "service_manager",
            "software_manager",
            "host_manager",
            "settings_manager",
            "command_manager",
            "system_administrator",
        ]
    );

    // Scope presets.
    let all_reads = catalog
        .scope_presets
        .iter()
        .find(|p| p.name == "all-reads")
        .expect("all-reads preset present");
    assert_eq!(all_reads.kind, ScopePresetKind::Static);
    let all_reads_actions = all_reads.actions.as_ref().expect("all-reads has actions");
    assert!(all_reads_actions.iter().any(|a| a == "hosts:read"));
    assert!(!all_reads_actions.iter().any(|a| a == "updates:trigger"));

    let all_my_current = catalog
        .scope_presets
        .iter()
        .find(|p| p.name == "all-my-current-actions")
        .expect("all-my-current-actions preset present");
    assert_eq!(all_my_current.kind, ScopePresetKind::CallerActions);
    assert!(all_my_current.actions.is_none());
}

/// Reach the registry directly (`TestApp::new()` — never
/// `with_stub_surfaces`, whose fresh post-swap registry is invisible to the
/// `AccessEngine` the app was wired with) and flip a `surface.test.stub`
/// registration on and off, proving the catalog's dynamic-action section
/// tracks live registry state both ways.
#[tokio::test]
async fn dynamic_actions_appear_and_disappear_with_registry_state() {
    let app = TestApp::new().await;
    let client = app.client();
    let (_, token) = stage_zero_role_user(&app).await;
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

    let (status, catalog): (StatusCode, AccessCatalogResponse) = client
        .get("/api/v1/access/catalog")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);

    let stub_entry = catalog
        .resources
        .iter()
        .find(|r| r.resource == "surface.test.stub")
        .expect("surface.test.stub resource present while registered");
    assert_eq!(stub_entry.actions.len(), 1);
    assert_eq!(stub_entry.actions[0].action, "surface.test.stub:use");
    assert_eq!(stub_entry.actions[0].verb, "use");
    assert_eq!(
        stub_entry.actions[0].selector_support,
        SelectorSupport::None
    );

    registry.unregister_service(&service_id);

    let (status, catalog): (StatusCode, AccessCatalogResponse) = client
        .get("/api/v1/access/catalog")
        .bearer(&token)
        .send_json()
        .await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        !catalog
            .resources
            .iter()
            .any(|r| r.resource == "surface.test.stub"),
        "surface.test.stub must disappear once unregistered"
    );
}
