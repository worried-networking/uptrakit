use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::{RwLock, RwLockWriteGuard};

use crate::auth;
use crate::auth::registration::RegistrationSettings;

#[derive(Clone)]
pub struct Settings {
    inner: Arc<Inner>,
}

struct Inner {
    registration: RwLock<RegistrationSettings>,
}

impl Settings {
    /// Construct from pre-loaded values (for tests).
    pub fn new(registration: RegistrationSettings) -> Self {
        Self {
            inner: Arc::new(Inner {
                registration: RwLock::new(registration),
            }),
        }
    }

    /// Load all settings from DB. Generates initial registration token
    /// if no users exist. Returns `(Settings, Option<plaintext_token>)`.
    pub async fn load(db: &DatabaseConnection) -> auth::Result<(Self, Option<String>)> {
        let (registration, token) = RegistrationSettings::initialize(db).await?;
        Ok((Self::new(registration), token))
    }

    /// Read registration settings (acquires read lock, returns clone).
    pub async fn registration(&self) -> RegistrationSettings {
        self.inner.registration.read().await.clone()
    }

    /// Acquire write access to registration settings.
    pub async fn registration_write(&self) -> RwLockWriteGuard<'_, RegistrationSettings> {
        self.inner.registration.write().await
    }
}
