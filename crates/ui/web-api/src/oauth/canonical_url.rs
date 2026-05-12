//! Re-exports of [`CanonicalUrlConfig`] and related types from the shared crate,
//! plus a disabled-mode placeholder constructor for use in [`super::OAuthState::disabled`],
//! and the boot-time settings-to-config resolver.

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;

pub use uptrakit_web_api_types::oauth::{
    CanonicalUrlConfig, CanonicalUrlConfigError, CanonicalUrlError, MAX_ACCEPTED_AUDIENCE_HOSTS,
};

/// Returns a [`CanonicalUrlConfig`] that is only valid as a placeholder when OAuth is
/// disabled.
///
/// Uses `disabled.invalid` — a reserved TLD that can never resolve — so that
/// callers that accidentally reach OAuth logic while `enabled = false` fail loudly
/// rather than silently accepting every request.
///
/// # Panics
///
/// This constructor is called only when `oauth.mcp_enabled = false`.  The host
/// `disabled.invalid` satisfies all canonicalisation rules, so the inner
/// `CanonicalUrlConfig::new` call will always succeed.  A panic here indicates
/// a bug in the canonicalisation rules, not a configuration error.
#[must_use]
pub fn disabled_placeholder() -> CanonicalUrlConfig {
    #[expect(
        clippy::expect_used,
        reason = "disabled.invalid always satisfies canonicalisation rules; \
                  a panic here indicates a bug in CanonicalUrlConfig::new, not a config error"
    )]
    CanonicalUrlConfig::new("disabled.invalid".to_string(), vec![])
        .expect("disabled placeholder is always valid")
}

/// Load [`CanonicalUrlConfig`] from `global_settings` at boot time.
///
/// Reads `oauth.canonical_host` and `oauth.accepted_audience_hosts` (a JSON
/// string array) from the `global_settings` table and constructs a fully
/// validated [`CanonicalUrlConfig`].
///
/// # Errors
///
/// Returns [`CanonicalUrlConfigError::Missing`] if `oauth.canonical_host` is unset or empty.
/// Returns [`CanonicalUrlConfigError::TooManyAliases`] if more than
/// [`MAX_ACCEPTED_AUDIENCE_HOSTS`] aliases were supplied.
/// Returns [`CanonicalUrlConfigError::InvalidHost`] if any host fails canonicalisation.
pub async fn load_canonical_url_config(
    db: &DatabaseConnection,
) -> Result<CanonicalUrlConfig, Report<CanonicalUrlConfigError>> {
    use sea_orm::EntityTrait;
    use uptrakit_shared_db::entity::prelude::GlobalSetting;

    let host_row = GlobalSetting::find_by_id("oauth.canonical_host".to_string())
        .one(db)
        .await
        .map_err(|e| report!(CanonicalUrlConfigError::Missing).attach(format!("db error: {e}")))?;

    let canonical_host = host_row
        .and_then(|r| r.value.as_str().map(str::to_owned))
        .unwrap_or_default();

    let aliases_row = GlobalSetting::find_by_id("oauth.accepted_audience_hosts".to_string())
        .one(db)
        .await
        .map_err(|e| report!(CanonicalUrlConfigError::Missing).attach(format!("db error: {e}")))?;

    let aliases_json = aliases_row
        .and_then(|r| r.value.as_array().cloned())
        .unwrap_or_default();

    let aliases: Vec<String> = aliases_json
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();

    CanonicalUrlConfig::new(canonical_host, aliases).map_err(|e| report!(e))
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::test_harness::setup_migrated_db;
    use uptrakit_shared_db::raw_settings::upsert_global_setting_raw;

    #[tokio::test]
    async fn resolves_config_from_settings() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(
            &db,
            "oauth.canonical_host",
            serde_json::json!("controller.example.com"),
        )
        .await
        .expect("insert canonical_host");

        let cfg = load_canonical_url_config(&db)
            .await
            .expect("load_canonical_url_config should succeed");

        assert_eq!(cfg.issuer().as_str(), "https://controller.example.com");
        assert_eq!(
            cfg.primary_resource().as_str(),
            "https://controller.example.com/mcp"
        );
        assert!(cfg.accepts_audience("https://controller.example.com/mcp"));
    }

    #[tokio::test]
    async fn missing_canonical_host_bails() {
        let db = setup_migrated_db().await;
        // No oauth.canonical_host row inserted — expect Missing error.
        let err = load_canonical_url_config(&db)
            .await
            .expect_err("should fail when canonical_host is absent");

        assert!(matches!(
            err.current_context(),
            CanonicalUrlConfigError::Missing
        ));
    }

    #[tokio::test]
    async fn parses_aliases_from_json_array() {
        let db = setup_migrated_db().await;
        upsert_global_setting_raw(
            &db,
            "oauth.canonical_host",
            serde_json::json!("controller.example.com"),
        )
        .await
        .expect("insert canonical_host");
        upsert_global_setting_raw(
            &db,
            "oauth.accepted_audience_hosts",
            serde_json::json!(["legacy.example.com"]),
        )
        .await
        .expect("insert accepted_audience_hosts");

        let cfg = load_canonical_url_config(&db)
            .await
            .expect("load_canonical_url_config should succeed");

        assert!(cfg.accepts_audience("https://controller.example.com/mcp"));
        assert!(cfg.accepts_audience("https://legacy.example.com/mcp"));
        assert!(!cfg.accepts_audience("https://intruder.example.com/mcp"));
    }
}
