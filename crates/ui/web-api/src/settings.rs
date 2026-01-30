use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::{RwLock, RwLockWriteGuard};

use crate::auth;
use crate::auth::registration::RegistrationSettings;
use crate::settings_store::load_setting;

const SETTING_KEY_AGENT_CERT_LIFETIME: &str = "agent_certificate.lifetime_days";
const DEFAULT_AGENT_CERT_LIFETIME_DAYS: u16 = 7;

#[derive(Clone)]
pub struct Settings {
    inner: Arc<Inner>,
}

struct Inner {
    registration: RwLock<RegistrationSettings>,
    agent_cert_lifetime_days: RwLock<u16>,
}

impl Settings {
    /// Construct from pre-loaded values (for tests).
    pub fn new(registration: RegistrationSettings, agent_cert_lifetime_days: u16) -> Self {
        Self {
            inner: Arc::new(Inner {
                registration: RwLock::new(registration),
                agent_cert_lifetime_days: RwLock::new(agent_cert_lifetime_days),
            }),
        }
    }

    /// Load all settings from DB. Generates initial registration token
    /// if no users exist. Returns `(Settings, Option<plaintext_token>)`.
    pub async fn load(db: &DatabaseConnection) -> auth::Result<(Self, Option<String>)> {
        let (registration, token) = RegistrationSettings::initialize(db).await?;

        let agent_cert_lifetime_days = match load_setting(db, SETTING_KEY_AGENT_CERT_LIFETIME).await
        {
            Ok(Some(v)) => v.parse::<u16>().unwrap_or(DEFAULT_AGENT_CERT_LIFETIME_DAYS),
            _ => DEFAULT_AGENT_CERT_LIFETIME_DAYS,
        };

        Ok((
            Self::new(registration, agent_cert_lifetime_days),
            token,
        ))
    }

    /// Read registration settings (acquires read lock, returns clone).
    pub async fn registration(&self) -> RegistrationSettings {
        self.inner.registration.read().await.clone()
    }

    /// Acquire write access to registration settings.
    pub async fn registration_write(&self) -> RwLockWriteGuard<'_, RegistrationSettings> {
        self.inner.registration.write().await
    }

    /// Read the agent certificate lifetime in days.
    pub async fn agent_cert_lifetime_days(&self) -> u16 {
        *self.inner.agent_cert_lifetime_days.read().await
    }

    /// Update the agent certificate lifetime in days.
    pub async fn set_agent_cert_lifetime_days(&self, days: u16) {
        *self.inner.agent_cert_lifetime_days.write().await = days;
    }
}
