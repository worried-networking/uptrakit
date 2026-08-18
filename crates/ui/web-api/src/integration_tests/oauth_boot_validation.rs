// Phase 1.5 boot validation tests — per spec §7 + §20 + §24.
//
// Task 23: boot fails when canonical_host missing.
// Task 24: minimal config boot succeeds; multi-controller guard rejects a
//          different fingerprint.

use http::StatusCode;

use crate::oauth::boot::{
    OAuthBootError, OAuthBootSettings, boot_oauth_state, validate_and_register,
};
use crate::settings_store::upsert_global_setting_raw;
use crate::test_harness::TestApp;
use crate::test_harness::fixtures::stage_user_with_grant;
use crate::test_harness::{insert_default_tenant, setup_migrated_db};

// ── Task 23 ─────────────────────────────────────────────────────────────────

/// Boot must fail with [`OAuthBootError::CanonicalHostMissing`] when
/// `canonical_host` is `None`.
#[tokio::test]
async fn boot_fails_when_canonical_host_missing() {
    let db = setup_migrated_db().await;
    let _ = insert_default_tenant(&db).await;
    let settings = OAuthBootSettings::new(None, vec![], b"secret".to_vec(), false);
    let err = validate_and_register(&db, &settings, time::OffsetDateTime::now_utc())
        .await
        .unwrap_err();
    assert!(
        matches!(err.current_context(), OAuthBootError::CanonicalHostMissing),
        "unexpected error: {err:?}",
    );
}

// ── Task 24 ─────────────────────────────────────────────────────────────────

/// A minimal valid configuration should boot successfully and return a non-nil
/// instance UUID.
#[tokio::test]
async fn minimal_config_boot_succeeds() {
    let db = setup_migrated_db().await;
    let _ = insert_default_tenant(&db).await;
    let settings = OAuthBootSettings::new(
        Some("test.example.com".to_string()),
        vec![],
        b"secret".to_vec(),
        false,
    );
    let id = validate_and_register(&db, &settings, time::OffsetDateTime::now_utc())
        .await
        .expect("minimal config should boot successfully");
    assert!(!id.is_nil(), "returned instance_id must not be nil");
}

/// When a controller with a different signing secret is already active,
/// [`validate_and_register`] must reject boot with
/// [`OAuthBootError::PeerWithDifferentFingerprint`].
#[tokio::test]
async fn multi_controller_guard_rejects_different_fingerprint() {
    let db = setup_migrated_db().await;
    let _ = insert_default_tenant(&db).await;
    let now = time::OffsetDateTime::now_utc();

    let settings1 = OAuthBootSettings::new(
        Some("test.example.com".to_string()),
        vec![],
        b"secret-a".to_vec(),
        false,
    );
    validate_and_register(&db, &settings1, now)
        .await
        .expect("first boot should succeed");

    let settings2 = OAuthBootSettings::new(
        Some("test.example.com".to_string()),
        vec![],
        b"secret-b".to_vec(),
        false,
    );
    let err = validate_and_register(&db, &settings2, now + time::Duration::seconds(5))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.current_context(),
            OAuthBootError::PeerWithDifferentFingerprint
        ),
        "unexpected error: {err:?}",
    );
}

// ── boot_oauth_state tests ───────────────────────────────────────────────────

/// When `canonical_host` is set but no `mcp_enabled` row exists,
/// `boot_oauth_state` must return a disabled `OAuthState` — enabling MCP
/// always requires an explicit `oauth.mcp_enabled = true` row.
#[tokio::test]
async fn boot_oauth_state_stays_disabled_when_only_canonical_host_set() {
    uptrakit_crypto::enable_plaintext_mode();
    let db = setup_migrated_db().await;
    let _ = insert_default_tenant(&db).await;
    upsert_global_setting_raw(
        &db,
        "oauth.canonical_host",
        serde_json::json!("example.com"),
    )
    .await
    .expect("write canonical_host");
    // No mcp_enabled row — must stay disabled (no auto-enable).
    let state = boot_oauth_state(&db)
        .await
        .expect("boot_oauth_state must succeed");
    assert!(!state.enabled, "OAuthState must stay disabled");
}

/// When both `canonical_host` and an explicit `mcp_enabled = true` row are
/// set, `boot_oauth_state` must return a live (enabled) `OAuthState`.
#[tokio::test]
async fn boot_oauth_state_enables_with_explicit_row() {
    uptrakit_crypto::enable_plaintext_mode();
    let db = setup_migrated_db().await;
    let _ = insert_default_tenant(&db).await;
    upsert_global_setting_raw(
        &db,
        "oauth.canonical_host",
        serde_json::json!("example.com"),
    )
    .await
    .expect("write canonical_host");
    upsert_global_setting_raw(&db, "oauth.mcp_enabled", serde_json::json!(true))
        .await
        .expect("write mcp_enabled");
    let state = boot_oauth_state(&db)
        .await
        .expect("boot_oauth_state must succeed");
    assert!(state.enabled, "OAuthState must be enabled");
    assert!(!state.instance_id.is_nil(), "instance_id must not be nil");
}

/// When no `canonical_host` is set and no `mcp_enabled` row exists,
/// `boot_oauth_state` returns a disabled `OAuthState`.
#[tokio::test]
async fn boot_oauth_state_disabled_when_no_host_and_no_explicit_flag() {
    uptrakit_crypto::enable_plaintext_mode();
    let db = setup_migrated_db().await;
    let _ = insert_default_tenant(&db).await;
    let state = boot_oauth_state(&db)
        .await
        .expect("boot_oauth_state must succeed");
    assert!(!state.enabled, "OAuthState must be disabled");
}

// ── GET /api/v1/global-settings/oauth pins the inverted resolver ───────────

/// Pins the third `resolve_mcp_enabled` call site
/// (`load_oauth_settings_from_db` in `routes/settings_oauth.rs`): with only
/// `oauth.canonical_host` set and no explicit `oauth.mcp_enabled` row, the
/// settings-report endpoint must report `mcp_enabled: false`. The in-memory
/// test-state `OAuthState` is `OAuthState::disabled()`, so `restart_required`
/// (persisted-vs-boot mismatch) is also `false`.
#[tokio::test]
async fn oauth_settings_report_disabled_without_explicit_row() {
    uptrakit_crypto::enable_plaintext_mode();
    let app = TestApp::new().await;
    upsert_global_setting_raw(
        &app.db,
        "oauth.canonical_host",
        serde_json::json!("example.com"),
    )
    .await
    .expect("write canonical_host");

    // `system.settings:manage` is a system-plane action: `insert_grant`
    // rejects a non-NULL `tenant_id` for it (`TenantEncoding` error).
    let (_user_id, token) =
        stage_user_with_grant(&app, "sys@example.com", &["system.settings:manage"], None).await;
    let client = app.client();

    let (status, body): (StatusCode, serde_json::Value) = client
        .get("/api/v1/global-settings/oauth")
        .bearer(&token)
        .send_json()
        .await;

    assert_eq!(status, StatusCode::OK, "unexpected status: {body:?}");
    assert_eq!(
        body.get("mcp_enabled"),
        Some(&serde_json::json!(false)),
        "mcp_enabled must stay false without an explicit oauth.mcp_enabled row: {body:?}"
    );
    assert_eq!(
        body.get("restart_required"),
        Some(&serde_json::json!(false)),
        "restart_required must be false when persisted state matches boot-time disabled state: {body:?}"
    );
}
