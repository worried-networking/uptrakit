use super::{Result, password, token::generate_secure_token};
use crate::SettingKey;
use crate::settings_store::{
    RawSettings, RawSettingsExt, delete_setting, load_setting, upsert_setting,
};
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use rootcause::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait};
use thiserror::Error;
use uptrakit_shared_db::entity::prelude::*;
use uuid::Uuid;

use crate::error_response::error_response;

pub use uptrakit_web_api_types::registration::RegistrationMode;

/// Error returned when registration validation fails.
#[derive(Debug, Error)]
pub enum RegistrationValidationError {
    #[error("registration is currently closed")]
    Closed,
    #[error("registration token is required")]
    TokenRequired,
    #[error("no registration token configured")]
    NoTokenConfigured,
    #[error("token verification failed")]
    VerificationFailed,
    #[error("invalid registration token")]
    InvalidToken,
}

impl IntoResponse for RegistrationValidationError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::VerificationFailed => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Closed | Self::TokenRequired | Self::NoTokenConfigured | Self::InvalidToken => {
                StatusCode::FORBIDDEN
            }
        };
        error_response(status, self.to_string())
    }
}

/// Cached registration settings held in AppState.
#[derive(Clone, Debug)]
pub struct RegistrationSettings {
    pub mode: RegistrationMode,
    pub token_hash: Option<String>,
    /// When true, OIDC users also need a registration token to register
    /// (only relevant in `Invite` mode).
    pub require_token_for_oidc: bool,
}

impl RegistrationSettings {
    /// Build from pre-fetched settings map. No DB access required.
    pub fn from_raw(raw: &RawSettings) -> Self {
        let mode = raw
            .get_setting(SettingKey::RegistrationMode)
            .and_then(|v| v.as_str().and_then(|s| s.parse::<RegistrationMode>().ok()))
            .unwrap_or(RegistrationMode::Closed);

        let token_hash = raw
            .get_setting(SettingKey::RegistrationTokenHash)
            .and_then(|v| v.as_str().map(String::from));

        let require_token_for_oidc = raw
            .get_setting(SettingKey::RegistrationRequireTokenForOidc)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Self {
            mode,
            token_hash,
            require_token_for_oidc,
        }
    }

    /// Load from DB or generate initial invite token (if no users exist).
    ///
    /// - If no users exist: generate a new invite token, store/overwrite it, return
    ///   settings + plaintext token for logging.
    /// - If users exist: load existing settings from the pre-fetched map
    ///   (defaulting to Closed if absent).
    pub async fn initialize(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        raw: &RawSettings,
    ) -> Result<(Self, Option<String>)> {
        let user_count = User::find().count(db).await.context_to()?;

        if user_count == 0 {
            // Generate a new invite token
            let plaintext = generate_secure_token()?;
            let hash = password::hash_password(&plaintext)?;

            // Upsert mode = invite
            upsert_setting(
                db,
                tenant_id,
                SettingKey::RegistrationMode,
                serde_json::Value::String(RegistrationMode::Invite.as_str().to_string()),
            )
            .await?;
            // Upsert token hash
            let hash_str = hash.expose_secret().to_string();
            upsert_setting(
                db,
                tenant_id,
                SettingKey::RegistrationTokenHash,
                serde_json::Value::String(hash_str.clone()),
            )
            .await?;

            let settings = RegistrationSettings {
                mode: RegistrationMode::Invite,
                token_hash: Some(hash_str),
                require_token_for_oidc: false,
            };
            Ok((settings, Some(plaintext)))
        } else {
            Ok((Self::from_raw(raw), None))
        }
    }

    /// Validate a registration attempt against current mode/token.
    ///
    /// - `Open`: always allowed, token ignored.
    /// - `Closed`: always rejected.
    /// - `Invite`: token must be present and match the stored hash.
    pub fn validate(
        &self,
        token: Option<&str>,
    ) -> std::result::Result<(), RegistrationValidationError> {
        match self.mode {
            RegistrationMode::Open => Ok(()),
            RegistrationMode::Closed => Err(RegistrationValidationError::Closed),
            RegistrationMode::Invite => {
                let token = token.ok_or(RegistrationValidationError::TokenRequired)?;
                let stored_hash = self
                    .token_hash
                    .as_deref()
                    .ok_or(RegistrationValidationError::NoTokenConfigured)?;
                let valid = password::verify_password(token, stored_hash).map_err(|e| {
                    tracing::error!("Token verification error: {:?}", e);
                    RegistrationValidationError::VerificationFailed
                })?;
                if valid {
                    Ok(())
                } else {
                    Err(RegistrationValidationError::InvalidToken)
                }
            }
            _ => Err(RegistrationValidationError::Closed),
        }
    }

    /// First admin registered — set mode to Closed, clear token hash.
    /// Updates both DB and in-memory state.
    pub async fn complete_initial_setup(
        &mut self,
        db: &impl ConnectionTrait,
        tenant_id: Uuid,
    ) -> Result<()> {
        // Update DB
        upsert_setting(
            db,
            tenant_id,
            SettingKey::RegistrationMode,
            serde_json::Value::String(RegistrationMode::Closed.as_str().to_string()),
        )
        .await?;
        delete_setting(db, tenant_id, SettingKey::RegistrationTokenHash).await?;

        delete_setting(db, tenant_id, SettingKey::RegistrationRequireTokenForOidc).await?;

        // Update in-memory state
        self.mode = RegistrationMode::Closed;
        self.token_hash = None;
        self.require_token_for_oidc = false;

        Ok(())
    }

    /// Returns `true` if OIDC registration requires a token.
    ///
    /// - Always requires token for the first user (initial setup).
    /// - For subsequent users, requires token only if `require_token_for_oidc` is enabled.
    pub fn needs_token_for_oidc(&self, is_first_user: bool) -> bool {
        self.mode == RegistrationMode::Invite && (is_first_user || self.require_token_for_oidc)
    }

    /// Admin action — change mode and/or set a new token.
    /// Updates both DB and in-memory state.
    pub async fn update(
        &mut self,
        db: &DatabaseConnection,
        tenant_id: Uuid,
        mode: RegistrationMode,
        token: Option<String>,
        require_token_for_oidc: Option<bool>,
    ) -> Result<()> {
        // Update mode in DB
        upsert_setting(
            db,
            tenant_id,
            SettingKey::RegistrationMode,
            serde_json::Value::String(mode.as_str().to_string()),
        )
        .await?;

        // Handle token
        let token_hash = if mode == RegistrationMode::Invite {
            if let Some(ref plaintext) = token {
                let hash = password::hash_password(plaintext)?;
                let hash_str = hash.expose_secret().to_string();
                upsert_setting(
                    db,
                    tenant_id,
                    SettingKey::RegistrationTokenHash,
                    serde_json::Value::String(hash_str.clone()),
                )
                .await?;
                Some(hash_str)
            } else {
                // Keep existing hash if no new token provided (shouldn't happen per API contract,
                // but handled defensively)
                load_setting(db, tenant_id, SettingKey::RegistrationTokenHash)
                    .await?
                    .and_then(|v| v.as_str().map(String::from))
            }
        } else {
            // Open or Closed: clear any stored token
            delete_setting(db, tenant_id, SettingKey::RegistrationTokenHash).await?;
            None
        };

        // Handle require_token_for_oidc
        let require_oidc = if mode == RegistrationMode::Invite {
            if let Some(val) = require_token_for_oidc {
                upsert_setting(
                    db,
                    tenant_id,
                    SettingKey::RegistrationRequireTokenForOidc,
                    serde_json::Value::Bool(val),
                )
                .await?;
                val
            } else {
                self.require_token_for_oidc
            }
        } else {
            // Not invite mode: clear the flag
            delete_setting(db, tenant_id, SettingKey::RegistrationRequireTokenForOidc).await?;
            false
        };

        // Update in-memory state
        self.mode = mode;
        self.token_hash = token_hash;
        self.require_token_for_oidc = require_oidc;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_store::RawSettings;

    fn make_raw(entries: &[(&str, serde_json::Value)]) -> RawSettings {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    // ── from_raw ─────────────────────────────────────────────────────

    #[test]
    fn from_raw_defaults_to_closed_when_empty() {
        let raw = RawSettings::new();
        let settings = RegistrationSettings::from_raw(&raw);
        assert_eq!(settings.mode, RegistrationMode::Closed);
        assert!(settings.token_hash.is_none());
        assert!(!settings.require_token_for_oidc);
    }

    #[test]
    fn from_raw_reads_open_mode() {
        let raw = make_raw(&[("registration.mode", serde_json::json!("open"))]);
        let settings = RegistrationSettings::from_raw(&raw);
        assert_eq!(settings.mode, RegistrationMode::Open);
    }

    #[test]
    fn from_raw_reads_invite_mode_with_token() {
        let raw = make_raw(&[
            ("registration.mode", serde_json::json!("invite")),
            (
                "registration.token_hash",
                serde_json::json!("$argon2id$test_hash"),
            ),
        ]);
        let settings = RegistrationSettings::from_raw(&raw);
        assert_eq!(settings.mode, RegistrationMode::Invite);
        assert_eq!(settings.token_hash.as_deref(), Some("$argon2id$test_hash"));
    }

    #[test]
    fn from_raw_reads_require_token_for_oidc() {
        let raw = make_raw(&[
            ("registration.mode", serde_json::json!("invite")),
            (
                "registration.require_token_for_oidc",
                serde_json::json!(true),
            ),
        ]);
        let settings = RegistrationSettings::from_raw(&raw);
        assert!(settings.require_token_for_oidc);
    }

    #[test]
    fn from_raw_invalid_mode_defaults_to_closed() {
        let raw = make_raw(&[("registration.mode", serde_json::json!("invalid_mode"))]);
        let settings = RegistrationSettings::from_raw(&raw);
        assert_eq!(settings.mode, RegistrationMode::Closed);
    }

    #[test]
    fn from_raw_non_string_mode_defaults_to_closed() {
        let raw = make_raw(&[("registration.mode", serde_json::json!(42))]);
        let settings = RegistrationSettings::from_raw(&raw);
        assert_eq!(settings.mode, RegistrationMode::Closed);
    }

    // ── validate ─────────────────────────────────────────────────────

    #[test]
    fn validate_open_mode_always_succeeds() {
        let settings = RegistrationSettings {
            mode: RegistrationMode::Open,
            token_hash: None,
            require_token_for_oidc: false,
        };
        assert!(settings.validate(None).is_ok());
        assert!(settings.validate(Some("any_token")).is_ok());
    }

    #[test]
    fn validate_closed_mode_always_fails() {
        let settings = RegistrationSettings {
            mode: RegistrationMode::Closed,
            token_hash: None,
            require_token_for_oidc: false,
        };
        let err = settings.validate(None).unwrap_err();
        assert!(matches!(err, RegistrationValidationError::Closed));
    }

    #[test]
    fn validate_invite_mode_requires_token() {
        let hash = password::hash_password("valid_token").expect("hash");
        let settings = RegistrationSettings {
            mode: RegistrationMode::Invite,
            token_hash: Some(hash.expose_secret().to_string()),
            require_token_for_oidc: false,
        };
        let err = settings.validate(None).unwrap_err();
        assert!(matches!(err, RegistrationValidationError::TokenRequired));
    }

    #[test]
    fn validate_invite_mode_no_hash_configured() {
        let settings = RegistrationSettings {
            mode: RegistrationMode::Invite,
            token_hash: None,
            require_token_for_oidc: false,
        };
        let err = settings.validate(Some("any_token")).unwrap_err();
        assert!(matches!(
            err,
            RegistrationValidationError::NoTokenConfigured
        ));
    }

    #[test]
    fn validate_invite_mode_correct_token() {
        let hash = password::hash_password("valid_token").expect("hash");
        let settings = RegistrationSettings {
            mode: RegistrationMode::Invite,
            token_hash: Some(hash.expose_secret().to_string()),
            require_token_for_oidc: false,
        };
        assert!(settings.validate(Some("valid_token")).is_ok());
    }

    #[test]
    fn validate_invite_mode_wrong_token() {
        let hash = password::hash_password("valid_token").expect("hash");
        let settings = RegistrationSettings {
            mode: RegistrationMode::Invite,
            token_hash: Some(hash.expose_secret().to_string()),
            require_token_for_oidc: false,
        };
        let err = settings.validate(Some("wrong_token")).unwrap_err();
        assert!(matches!(err, RegistrationValidationError::InvalidToken));
    }

    // ── needs_token_for_oidc ─────────────────────────────────────────

    #[test]
    fn needs_token_for_oidc_false_when_not_invite() {
        let settings = RegistrationSettings {
            mode: RegistrationMode::Open,
            token_hash: None,
            require_token_for_oidc: true,
        };
        assert!(!settings.needs_token_for_oidc(true));
        assert!(!settings.needs_token_for_oidc(false));
    }

    #[test]
    fn needs_token_for_oidc_true_for_first_user_in_invite() {
        let settings = RegistrationSettings {
            mode: RegistrationMode::Invite,
            token_hash: None,
            require_token_for_oidc: false,
        };
        assert!(settings.needs_token_for_oidc(true));
    }

    #[test]
    fn needs_token_for_oidc_true_when_flag_enabled() {
        let settings = RegistrationSettings {
            mode: RegistrationMode::Invite,
            token_hash: None,
            require_token_for_oidc: true,
        };
        assert!(settings.needs_token_for_oidc(false));
    }

    #[test]
    fn needs_token_for_oidc_false_when_not_first_and_flag_off() {
        let settings = RegistrationSettings {
            mode: RegistrationMode::Invite,
            token_hash: None,
            require_token_for_oidc: false,
        };
        assert!(!settings.needs_token_for_oidc(false));
    }

    // ── IntoResponse for RegistrationValidationError ─────────────────

    #[test]
    fn error_closed_returns_forbidden() {
        let resp = RegistrationValidationError::Closed.into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn error_verification_failed_returns_internal() {
        let resp = RegistrationValidationError::VerificationFailed.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn error_token_required_returns_forbidden() {
        let resp = RegistrationValidationError::TokenRequired.into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
