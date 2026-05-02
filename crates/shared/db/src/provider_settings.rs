use rootcause::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection};
use sha2::Digest;

/// Prefix for global GitHub provider settings keys.
pub const GITHUB_PROVIDER_PREFIX: &str = "global_provider_github.";

/// Canonical AAD for the GitHub provider auth token global setting.
pub const AAD_SETTINGS_GITHUB_PROVIDER_AUTH_TOKEN: &str =
    "uptrakit:settings:global_provider_github_auth_token";
pub const DEFAULT_GITHUB_API_BASE_URL: &str = "https://api.github.com";

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
    #[error("invalid GitHub provider settings: {0}")]
    InvalidConfiguration(String),
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

pub fn normalize_github_provider_defaults(
    defaults: GitHubProviderDefaults,
) -> Result<Option<GitHubProviderDefaults>> {
    let auth_token = defaults
        .auth_token
        .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()));
    let api_base_url = defaults
        .api_base_url
        .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()));

    match (auth_token, api_base_url) {
        (None, None) => Ok(None),
        (Some(auth_token), None) => Ok(Some(GitHubProviderDefaults {
            auth_token: Some(auth_token),
            api_base_url: Some(DEFAULT_GITHUB_API_BASE_URL.to_string()),
        })),
        (Some(auth_token), Some(api_base_url)) => {
            uptrakit_shared_types::validate_provider_api_base_url(&api_base_url).map_err(
                |err| report!(ProviderSettingsError::InvalidConfiguration(err.to_string())),
            )?;
            Ok(Some(GitHubProviderDefaults {
                auth_token: Some(auth_token),
                api_base_url: Some(api_base_url),
            }))
        }
        (None, Some(_)) => Err(report!(ProviderSettingsError::InvalidConfiguration(
            "api_base_url requires auth_token".to_string(),
        ))),
    }
}

pub fn github_provider_generation(defaults: &GitHubProviderDefaults) -> [u8; 32] {
    let mut hasher = sha2::Sha256::new();
    hasher.update(
        defaults
            .api_base_url
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.update([0]);
    hasher.update(
        defaults
            .auth_token
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.finalize().into()
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

#[cfg(test)]
mod tests {
    #![expect(
        clippy::let_underscore_must_use,
        reason = "test setup — ignoring Result from init_master_key which may already be initialized"
    )]

    use super::*;
    use crate::entity::global_setting;
    use sea_orm::{
        ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, Set,
    };
    use time::OffsetDateTime;

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

    #[test]
    fn provider_settings_normalize_trims_and_drops_blank_values() {
        let normalized = normalize_github_provider_defaults(GitHubProviderDefaults {
            auth_token: Some("  ghp_secret  ".to_string()),
            api_base_url: Some("  https://ghe.example.com/api/v3  ".to_string()),
        })
        .expect("normalize succeeds")
        .expect("record remains present");

        assert_eq!(normalized.auth_token.as_deref(), Some("ghp_secret"));
        assert_eq!(
            normalized.api_base_url.as_deref(),
            Some("https://ghe.example.com/api/v3")
        );
    }

    #[test]
    fn provider_settings_normalize_absent_record_returns_none() {
        let normalized = normalize_github_provider_defaults(GitHubProviderDefaults {
            auth_token: Some("   ".to_string()),
            api_base_url: None,
        })
        .expect("normalize succeeds");

        assert_eq!(normalized, None);
    }

    #[test]
    fn provider_settings_normalize_defaults_public_api_base_url_for_token_only() {
        let normalized = normalize_github_provider_defaults(GitHubProviderDefaults {
            auth_token: Some("ghp_secret".to_string()),
            api_base_url: None,
        })
        .expect("normalize succeeds")
        .expect("record remains present");

        assert_eq!(
            normalized.api_base_url.as_deref(),
            Some(DEFAULT_GITHUB_API_BASE_URL)
        );
    }

    #[test]
    fn provider_settings_normalize_rejects_custom_api_base_without_token() {
        let err = normalize_github_provider_defaults(GitHubProviderDefaults {
            auth_token: None,
            api_base_url: Some("https://ghe.example.com/api/v3".to_string()),
        })
        .expect_err("must reject custom URL without token");

        assert!(matches!(
            err.current_context(),
            ProviderSettingsError::InvalidConfiguration(_)
        ));
    }

    #[test]
    fn provider_settings_generation_changes_with_credentials() {
        let one = github_provider_generation(&GitHubProviderDefaults {
            auth_token: Some("ghp_one".to_string()),
            api_base_url: Some("https://api.github.com".to_string()),
        });
        let two = github_provider_generation(&GitHubProviderDefaults {
            auth_token: Some("ghp_two".to_string()),
            api_base_url: Some("https://api.github.com".to_string()),
        });

        assert_ne!(one, two);
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
