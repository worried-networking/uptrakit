use super::{Result, password, token::generate_secure_token};
use crate::SettingKey;
use crate::settings_store::{
    RawSettings, RawSettingsExt, delete_setting, load_setting, upsert_setting,
};
use rootcause::prelude::*;
use sea_orm::{DatabaseConnection, EntityTrait, PaginatorTrait};
use uptrakit_shared_db::entity::prelude::*;
use uuid::Uuid;

pub use uptrakit_web_api_types::registration::RegistrationMode;

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
            upsert_setting(
                db,
                tenant_id,
                SettingKey::RegistrationTokenHash,
                serde_json::Value::String(hash.clone()),
            )
            .await?;

            let settings = RegistrationSettings {
                mode: RegistrationMode::Invite,
                token_hash: Some(hash),
                require_token_for_oidc: false,
            };
            Ok((settings, Some(plaintext)))
        } else {
            // Read from pre-fetched map
            let mode = raw
                .get_setting(SettingKey::RegistrationMode)
                .and_then(|v| v.as_str())
                .and_then(RegistrationMode::parse_str)
                .unwrap_or(RegistrationMode::Closed);

            let token_hash = raw
                .get_setting(SettingKey::RegistrationTokenHash)
                .and_then(|v| v.as_str().map(String::from));

            let require_token_for_oidc = raw
                .get_setting(SettingKey::RegistrationRequireTokenForOidc)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let settings = RegistrationSettings {
                mode,
                token_hash,
                require_token_for_oidc,
            };
            Ok((settings, None))
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
    ) -> std::result::Result<(), (http::StatusCode, &'static str)> {
        match self.mode {
            RegistrationMode::Open => Ok(()),
            RegistrationMode::Closed => Err((
                http::StatusCode::FORBIDDEN,
                "Registration is currently closed",
            )),
            RegistrationMode::Invite => {
                let token = token.ok_or((
                    http::StatusCode::FORBIDDEN,
                    "Registration token is required",
                ))?;
                let stored_hash = self.token_hash.as_deref().ok_or((
                    http::StatusCode::FORBIDDEN,
                    "No registration token configured",
                ))?;
                let valid = password::verify_password(token, stored_hash).map_err(|e| {
                    tracing::error!("Token verification error: {:?}", e);
                    (
                        http::StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    )
                })?;
                if valid {
                    Ok(())
                } else {
                    Err((http::StatusCode::FORBIDDEN, "Invalid registration token"))
                }
            }
        }
    }

    /// First admin registered — set mode to Closed, clear token hash.
    /// Updates both DB and in-memory state.
    pub async fn complete_initial_setup(
        &mut self,
        db: &DatabaseConnection,
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
                upsert_setting(
                    db,
                    tenant_id,
                    SettingKey::RegistrationTokenHash,
                    serde_json::Value::String(hash.clone()),
                )
                .await?;
                Some(hash)
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
