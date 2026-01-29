use super::{Result, password, token::generate_secure_token};
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{prelude::*, setting};
use utoipa::ToSchema;

const SETTING_KEY_MODE: &str = "registration.mode";
const SETTING_KEY_TOKEN_HASH: &str = "registration.token_hash";

/// Registration mode controlling how new users can sign up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    /// Anyone can register without a token.
    Open,
    /// Registration requires a valid token.
    Invite,
    /// Registration is disabled.
    Closed,
}

impl RegistrationMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Invite => "invite",
            Self::Closed => "closed",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "invite" => Some(Self::Invite),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

/// Cached registration settings held in AppState.
#[derive(Clone, Debug)]
pub struct RegistrationSettings {
    pub mode: RegistrationMode,
    pub token_hash: Option<String>,
}

impl RegistrationSettings {
    /// Load from DB or generate initial invite token (if no users exist).
    ///
    /// - If no users exist: generate a new invite token, store/overwrite it, return
    ///   settings + plaintext token for logging.
    /// - If users exist: load existing settings from DB (defaulting to Closed if absent).
    pub async fn initialize(db: &DatabaseConnection) -> Result<(Self, Option<String>)> {
        let user_count = User::find().count(db).await.context_to()?;

        if user_count == 0 {
            // Generate a new invite token
            let plaintext = generate_secure_token()?;
            let hash = password::hash_password(&plaintext)?;

            // Upsert mode = invite
            upsert_setting(db, SETTING_KEY_MODE, RegistrationMode::Invite.as_str()).await?;
            // Upsert token hash
            upsert_setting(db, SETTING_KEY_TOKEN_HASH, &hash).await?;

            let settings = RegistrationSettings {
                mode: RegistrationMode::Invite,
                token_hash: Some(hash),
            };
            Ok((settings, Some(plaintext)))
        } else {
            // Load existing settings from DB
            let mode = load_setting(db, SETTING_KEY_MODE)
                .await?
                .and_then(|v| RegistrationMode::from_str(&v))
                .unwrap_or(RegistrationMode::Closed);

            let token_hash = load_setting(db, SETTING_KEY_TOKEN_HASH).await?;

            let settings = RegistrationSettings { mode, token_hash };
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
    pub async fn complete_initial_setup(&mut self, db: &DatabaseConnection) -> Result<()> {
        // Update DB
        upsert_setting(db, SETTING_KEY_MODE, RegistrationMode::Closed.as_str()).await?;
        delete_setting(db, SETTING_KEY_TOKEN_HASH).await?;

        // Update in-memory state
        self.mode = RegistrationMode::Closed;
        self.token_hash = None;

        Ok(())
    }

    /// Admin action — change mode and/or set a new token.
    /// Updates both DB and in-memory state.
    pub async fn update(
        &mut self,
        db: &DatabaseConnection,
        mode: RegistrationMode,
        token: Option<String>,
    ) -> Result<()> {
        // Update mode in DB
        upsert_setting(db, SETTING_KEY_MODE, mode.as_str()).await?;

        // Handle token
        let token_hash = if mode == RegistrationMode::Invite {
            if let Some(ref plaintext) = token {
                let hash = password::hash_password(plaintext)?;
                upsert_setting(db, SETTING_KEY_TOKEN_HASH, &hash).await?;
                Some(hash)
            } else {
                // Keep existing hash if no new token provided (shouldn't happen per API contract,
                // but handled defensively)
                load_setting(db, SETTING_KEY_TOKEN_HASH).await?
            }
        } else {
            // Open or Closed: clear any stored token
            delete_setting(db, SETTING_KEY_TOKEN_HASH).await?;
            None
        };

        // Update in-memory state
        self.mode = mode;
        self.token_hash = token_hash;

        Ok(())
    }
}

// --- DB helper functions ---

async fn upsert_setting(db: &DatabaseConnection, key: &str, value: &str) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let existing = Setting::find_by_id(key.to_string())
        .one(db)
        .await
        .context_to()?;

    if let Some(existing) = existing {
        let mut model: setting::ActiveModel = existing.into();
        model.value = Set(value.to_string());
        model.updated_at = Set(now);
        model.update(db).await.context_to()?;
    } else {
        let model = setting::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_string()),
            updated_at: Set(now),
        };
        model.insert(db).await.context_to()?;
    }

    Ok(())
}

async fn load_setting(db: &DatabaseConnection, key: &str) -> Result<Option<String>> {
    let setting = Setting::find_by_id(key.to_string())
        .one(db)
        .await
        .context_to()?;
    Ok(setting.map(|s| s.value))
}

async fn delete_setting(db: &DatabaseConnection, key: &str) -> Result<()> {
    Setting::delete_many()
        .filter(setting::Column::Key.eq(key))
        .exec(db)
        .await
        .context_to()?;
    Ok(())
}
