use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::{RwLock, RwLockWriteGuard};

use crate::auth;
use crate::auth::authentication::AuthenticationSettings;
use crate::auth::registration::RegistrationSettings;
use crate::settings_store::load_setting;

const SETTING_KEY_AGENT_CERT_LIFETIME: &str = "agent_certificate.lifetime_days";
const DEFAULT_AGENT_CERT_LIFETIME_DAYS: u16 = 7;

const SETTING_KEY_RENEWAL_WINDOW_HOURS: &str = "agent_certificate.renewal_window_hours";
const DEFAULT_RENEWAL_WINDOW_HOURS: u16 = 6;

#[derive(Clone)]
pub struct Settings {
    inner: Arc<Inner>,
}

struct Inner {
    registration: RwLock<RegistrationSettings>,
    authentication: RwLock<AuthenticationSettings>,
    agent_cert_lifetime_days: RwLock<u16>,
    renewal_window_hours: RwLock<u16>,
}

impl Settings {
    /// Construct from pre-loaded values (for tests).
    pub fn new(registration: RegistrationSettings, agent_cert_lifetime_days: u16) -> Self {
        Self::with_renewal_window(
            registration,
            agent_cert_lifetime_days,
            DEFAULT_RENEWAL_WINDOW_HOURS,
        )
    }

    /// Construct with all values (for tests or when loading from DB).
    pub fn with_renewal_window(
        registration: RegistrationSettings,
        agent_cert_lifetime_days: u16,
        renewal_window_hours: u16,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                registration: RwLock::new(registration),
                authentication: RwLock::new(AuthenticationSettings::default()),
                agent_cert_lifetime_days: RwLock::new(agent_cert_lifetime_days),
                renewal_window_hours: RwLock::new(renewal_window_hours),
            }),
        }
    }

    /// Load all settings from DB. Generates initial registration token
    /// if no users exist. Returns `(Settings, Option<plaintext_token>)`.
    pub async fn load(db: &DatabaseConnection) -> auth::Result<(Self, Option<String>)> {
        let (registration, token) = RegistrationSettings::initialize(db).await?;
        let authentication = AuthenticationSettings::load(db).await?;

        let agent_cert_lifetime_days = match load_setting(db, SETTING_KEY_AGENT_CERT_LIFETIME).await
        {
            Ok(Some(v)) => match v.as_u64().and_then(|n| u16::try_from(n).ok()) {
                Some(days) => days,
                None => DEFAULT_AGENT_CERT_LIFETIME_DAYS,
            },
            _ => DEFAULT_AGENT_CERT_LIFETIME_DAYS,
        };

        let renewal_window_hours = match load_setting(db, SETTING_KEY_RENEWAL_WINDOW_HOURS).await {
            Ok(Some(v)) => match v.as_u64().and_then(|n| u16::try_from(n).ok()) {
                Some(hours) => hours,
                None => DEFAULT_RENEWAL_WINDOW_HOURS,
            },
            _ => DEFAULT_RENEWAL_WINDOW_HOURS,
        };

        let settings = Self {
            inner: Arc::new(Inner {
                registration: RwLock::new(registration),
                authentication: RwLock::new(authentication),
                agent_cert_lifetime_days: RwLock::new(agent_cert_lifetime_days),
                renewal_window_hours: RwLock::new(renewal_window_hours),
            }),
        };

        Ok((settings, token))
    }

    /// Read registration settings (acquires read lock, returns clone).
    pub async fn registration(&self) -> RegistrationSettings {
        self.inner.registration.read().await.clone()
    }

    /// Acquire write access to registration settings.
    pub async fn registration_write(&self) -> RwLockWriteGuard<'_, RegistrationSettings> {
        self.inner.registration.write().await
    }

    /// Read authentication settings (acquires read lock, returns clone).
    pub async fn authentication(&self) -> AuthenticationSettings {
        self.inner.authentication.read().await.clone()
    }

    /// Acquire write access to authentication settings.
    pub async fn authentication_write(&self) -> RwLockWriteGuard<'_, AuthenticationSettings> {
        self.inner.authentication.write().await
    }

    /// Read the agent certificate lifetime in days.
    pub async fn agent_cert_lifetime_days(&self) -> u16 {
        *self.inner.agent_cert_lifetime_days.read().await
    }

    /// Update the agent certificate lifetime in days.
    pub async fn set_agent_cert_lifetime_days(&self, days: u16) {
        *self.inner.agent_cert_lifetime_days.write().await = days;
    }

    /// Read the certificate renewal window in hours.
    pub async fn renewal_window_hours(&self) -> u16 {
        *self.inner.renewal_window_hours.read().await
    }

    /// Update the certificate renewal window in hours.
    pub async fn set_renewal_window_hours(&self, hours: u16) {
        *self.inner.renewal_window_hours.write().await = hours;
    }
}
