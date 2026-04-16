use rootcause::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection};
use uptrakit_shared_types::{PluginTypeId, plugin_ids};

/// Prefix for global GitHub provider settings keys.
pub const GITHUB_PROVIDER_PREFIX: &str = "global_provider_github.";

/// Canonical AAD for the GitHub provider auth token global setting.
pub const AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN: &str =
    "uptrakit:settings:global_provider_github_auth_token";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalProviderSettingKey {
    GitHubAuthToken,
    GitHubApiBaseUrl,
}

impl GlobalProviderSettingKey {
    pub fn as_db_key(&self) -> &'static str {
        match self {
            Self::GitHubAuthToken => "global_provider_github.auth_token",
            Self::GitHubApiBaseUrl => "global_provider_github.api_base_url",
        }
    }

    pub fn from_db_key(key: &str) -> Option<Self> {
        match key {
            "global_provider_github.auth_token" => Some(Self::GitHubAuthToken),
            "global_provider_github.api_base_url" => Some(Self::GitHubApiBaseUrl),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitHubProviderDefaults {
    pub auth_token: Option<String>,
    pub api_base_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderSettingsError {
    #[error("settings storage error: {0}")]
    Storage(String),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("invalid stored setting value for {0}")]
    InvalidValue(&'static str),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<ProviderSettingsError>>;

uptrakit_shared_macros::impl_report_conversion!(
    crate::raw_settings::RawSettingsError => ProviderSettingsError,
    |e| ProviderSettingsError::Storage(e.to_string())
);
uptrakit_shared_macros::impl_report_conversion!(
    uptrakit_crypto::CryptoError => ProviderSettingsError,
    |e| ProviderSettingsError::Crypto(e.to_string())
);

pub async fn load_github_provider_defaults(
    db: &DatabaseConnection,
) -> Result<GitHubProviderDefaults> {
    let settings = crate::raw_settings::load_global_settings_by_prefix(db, GITHUB_PROVIDER_PREFIX)
        .await
        .context_to()?;

    let auth_token = match settings.get(GlobalProviderSettingKey::GitHubAuthToken.as_db_key()) {
        None => None,
        Some(value) => {
            let raw = value.as_str().ok_or_else(|| {
                report!(ProviderSettingsError::InvalidValue(
                    GlobalProviderSettingKey::GitHubAuthToken.as_db_key(),
                ))
            })?;
            if raw.is_empty() {
                None
            } else if uptrakit_crypto::is_encrypted(raw) {
                Some(
                    uptrakit_crypto::decrypt_str(raw, AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN)
                        .context_to()?,
                )
            } else {
                Some(raw.to_string())
            }
        }
    };

    let api_base_url = match settings.get(GlobalProviderSettingKey::GitHubApiBaseUrl.as_db_key()) {
        None => None,
        Some(value) => {
            let raw = value.as_str().ok_or_else(|| {
                report!(ProviderSettingsError::InvalidValue(
                    GlobalProviderSettingKey::GitHubApiBaseUrl.as_db_key(),
                ))
            })?;
            (!raw.is_empty()).then(|| raw.to_string())
        }
    };

    Ok(GitHubProviderDefaults {
        auth_token,
        api_base_url,
    })
}

pub async fn upsert_github_provider_defaults(
    db: &impl ConnectionTrait,
    auth_token: Option<&str>,
    api_base_url: Option<&str>,
) -> Result<()> {
    let stored_auth_token = match auth_token {
        Some("") | None => serde_json::json!(""),
        Some(token) => serde_json::json!(
            uptrakit_crypto::encrypt_str(token, AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN)
                .context_to()?
        ),
    };
    crate::raw_settings::upsert_global_setting_raw(
        db,
        GlobalProviderSettingKey::GitHubAuthToken.as_db_key(),
        stored_auth_token,
    )
    .await
    .context_to()?;

    crate::raw_settings::upsert_global_setting_raw(
        db,
        GlobalProviderSettingKey::GitHubApiBaseUrl.as_db_key(),
        serde_json::json!(api_base_url.unwrap_or("")),
    )
    .await
    .context_to()?;

    Ok(())
}

pub fn supports_github_provider_defaults(plugin_type_id: &PluginTypeId) -> bool {
    plugin_type_id == &plugin_ids::RELEASES_GITHUB
}

pub fn apply_github_provider_defaults_for_plugin(
    plugin_type_id: &PluginTypeId,
    local_config: &serde_json::Value,
    defaults: Option<&GitHubProviderDefaults>,
) -> serde_json::Value {
    if !supports_github_provider_defaults(plugin_type_id) {
        return local_config.clone();
    }

    let Some(defaults) = defaults else {
        return local_config.clone();
    };

    let mut merged = match local_config {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };

    apply_field_default(&mut merged, "auth_token", defaults.auth_token.as_deref());
    apply_field_default(
        &mut merged,
        "api_base_url",
        defaults.api_base_url.as_deref(),
    );

    serde_json::Value::Object(merged)
}

fn apply_field_default(
    config: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    default_value: Option<&str>,
) {
    let Some(default_value) = default_value else {
        return;
    };

    let should_apply = match config.get(field) {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(value)) => value.is_empty(),
        Some(_) => false,
    };

    if should_apply {
        config.insert(
            field.to_string(),
            serde_json::Value::String(default_value.to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::global_setting;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_shared_types::plugin_ids;

    async fn provider_settings_test_db() -> DatabaseConnection {
        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x24u8; 32]));
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1);
        let db = Database::connect(opt).await.expect("connect to test db");
        crate::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    #[test]
    fn provider_settings_key_round_trip() {
        for key in [
            GlobalProviderSettingKey::GitHubAuthToken,
            GlobalProviderSettingKey::GitHubApiBaseUrl,
        ] {
            let round_tripped =
                GlobalProviderSettingKey::from_db_key(key.as_db_key()).expect("must parse");
            assert_eq!(round_tripped, key);
        }
    }

    #[tokio::test]
    async fn provider_settings_blank_and_missing_fields_fallback() {
        let plugin_type = plugin_ids::RELEASES_GITHUB;
        let defaults = GitHubProviderDefaults {
            auth_token: Some("provider-token".to_string()),
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
        };
        let local = serde_json::json!({
            "auth_token": "",
            "api_base_url": null,
            "include_prereleases": true
        });

        let merged =
            apply_github_provider_defaults_for_plugin(&plugin_type, &local, Some(&defaults));
        assert_eq!(merged["auth_token"], "provider-token");
        assert_eq!(merged["api_base_url"], "https://ghe.example.com/api/v3");
        assert_eq!(merged["include_prereleases"], true);
    }

    #[tokio::test]
    async fn provider_settings_non_opt_in_plugin_bypass() {
        let plugin_type = plugin_ids::RELEASES_GITLAB;
        let defaults = GitHubProviderDefaults {
            auth_token: Some("provider-token".to_string()),
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
        };
        let local = serde_json::json!({
            "api_base_url": "",
        });

        let merged =
            apply_github_provider_defaults_for_plugin(&plugin_type, &local, Some(&defaults));
        assert_eq!(merged, local);
    }

    #[tokio::test]
    async fn provider_settings_auth_token_is_encrypted_at_rest() {
        let db = provider_settings_test_db().await;
        upsert_github_provider_defaults(
            &db,
            Some("ghp_provider_secret"),
            Some("https://ghe.example.com/api/v3"),
        )
        .await
        .expect("upsert provider settings");

        let row = crate::entity::prelude::GlobalSetting::find_by_id(
            GlobalProviderSettingKey::GitHubAuthToken
                .as_db_key()
                .to_string(),
        )
        .one(&db)
        .await
        .expect("query")
        .expect("row exists");
        let stored = row.value.as_str().expect("stored as string");
        assert!(
            uptrakit_crypto::is_encrypted(stored),
            "must be encrypted in DB"
        );
        assert_ne!(stored, "ghp_provider_secret");
    }

    #[tokio::test]
    async fn provider_settings_load_decrypts_auth_token() {
        let db = provider_settings_test_db().await;
        let now = OffsetDateTime::now_utc();

        let encrypted = uptrakit_crypto::encrypt_str(
            "ghp_provider_secret",
            AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN,
        )
        .expect("encrypt");

        global_setting::ActiveModel {
            key: Set(GlobalProviderSettingKey::GitHubAuthToken
                .as_db_key()
                .to_string()),
            value: Set(serde_json::json!(encrypted)),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert auth token");
        global_setting::ActiveModel {
            key: Set(GlobalProviderSettingKey::GitHubApiBaseUrl
                .as_db_key()
                .to_string()),
            value: Set(serde_json::json!("https://ghe.example.com/api/v3")),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert base url");

        let loaded = load_github_provider_defaults(&db)
            .await
            .expect("load defaults");
        assert_eq!(loaded.auth_token.as_deref(), Some("ghp_provider_secret"));
        assert_eq!(
            loaded.api_base_url.as_deref(),
            Some("https://ghe.example.com/api/v3")
        );
    }
}
