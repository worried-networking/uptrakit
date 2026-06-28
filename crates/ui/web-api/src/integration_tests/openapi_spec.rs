//! Golden-file test: `crates/ui/web-api/openapi.json` must equal the OpenAPI
//! document the server assembles. Regenerate with `UPDATE_OPENAPI=1`.
//!
//! In-crate integration test (reaches `crate::test_harness`). Run package-scoped
//! so the controller's `embed-frontend` feature is NOT pulled (no frontend build):
//!   cargo test -p uptrakit-web-api --all-features openapi_
//!   UPDATE_OPENAPI=1 cargo test -p uptrakit-web-api --all-features openapi_

use std::path::PathBuf;

use crate::router::build_router_with_openapi;
use crate::test_harness::TestApp;

fn ensure_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

fn openapi_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("openapi.json")
}

#[tokio::test]
async fn openapi_json_is_up_to_date() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let (_router, api) = build_router_with_openapi(app.state.clone());
    let generated = serde_json::to_string_pretty(&api).expect("serialize OpenAPI");

    let path = openapi_path();
    if std::env::var("UPDATE_OPENAPI").is_ok() {
        std::fs::write(&path, generated + "\n").expect("write openapi.json");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing {}; run UPDATE_OPENAPI=1 cargo test -p uptrakit-web-api --all-features openapi_",
            path.display()
        )
    });
    assert_eq!(
        committed.trim_end(),
        generated.trim_end(),
        "openapi.json is stale; regenerate with: UPDATE_OPENAPI=1 cargo test -p uptrakit-web-api --all-features openapi_"
    );
}

#[tokio::test]
async fn openapi_spec_eligible_endpoints_present() {
    ensure_crypto_provider();
    let app = TestApp::new().await;
    let (_router, api) = build_router_with_openapi(app.state.clone());
    let paths = api
        .paths
        .paths
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    // Representative spec-eligible REST endpoints across domains. If a handler is
    // moved to a post-split raw `.route()` (or its `routes!()` is dropped), this fails.
    for required in [
        "/api/v1/services",
        "/api/v1/hosts",
        "/api/v1/software-items",
        "/api/v1/plugin-type-settings",
        "/api/v1/users/{id}/password",
        "/api/v1/auth/email-change/confirm",
    ] {
        assert!(
            paths.contains(required),
            "spec missing required path: {required}"
        );
    }
}
