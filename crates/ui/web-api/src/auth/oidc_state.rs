use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use openidconnect::{Nonce, PkceCodeVerifier};
use time::OffsetDateTime;

use crate::routes::auth::UserResponse;

const TTL_SECONDS: i64 = 600; // 10 minutes
const EXCHANGE_TTL_SECONDS: i64 = 60; // 60 seconds

/// Pending OIDC authorization flow (stored between authorize and callback).
pub struct PendingOidcFlow {
    pub provider_id: uuid::Uuid,
    pub pkce_verifier: PkceCodeVerifier,
    pub nonce: Nonce,
    pub created_at: OffsetDateTime,
}

/// In-memory store for pending OIDC flows keyed by `state` parameter.
#[derive(Clone)]
pub struct OidcFlowStore {
    inner: Arc<Mutex<HashMap<String, PendingOidcFlow>>>,
}

impl Default for OidcFlowStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OidcFlowStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn insert(&self, state: String, flow: PendingOidcFlow) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .insert(state, flow);
    }

    pub fn take(&self, state: &str) -> Option<PendingOidcFlow> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .remove(state)
    }

    pub fn cleanup_expired(&self) {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(TTL_SECONDS);
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .retain(|_, flow| flow.created_at > cutoff);
    }
}

/// Pending account link (stored when OIDC callback requires user verification).
pub struct PendingAccountLink {
    pub provider_id: uuid::Uuid,
    pub oidc_subject: String,
    pub email: String,
    pub user_id: uuid::Uuid,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    /// Pre-mapped local role names to assign after linking.
    pub mapped_roles: Vec<String>,
    /// If the user is linked to another active OIDC provider, this is set.
    pub existing_link_provider_id: Option<uuid::Uuid>,
    pub created_at: OffsetDateTime,
}

/// In-memory store for pending account links keyed by a random token.
#[derive(Clone)]
pub struct AccountLinkStore {
    inner: Arc<Mutex<HashMap<String, PendingAccountLink>>>,
}

impl Default for AccountLinkStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountLinkStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn insert(&self, token: String, link: PendingAccountLink) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .insert(token, link);
    }

    pub fn take(&self, token: &str) -> Option<PendingAccountLink> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .remove(token)
    }

    pub fn cleanup_expired(&self) {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(TTL_SECONDS);
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .retain(|_, link| link.created_at > cutoff);
    }
}

/// Pending OIDC token exchange (stored between callback redirect and exchange API call).
pub struct PendingOidcTokenExchange {
    pub access_token: String,
    pub refresh_token: String,
    pub user: UserResponse,
    pub created_at: OffsetDateTime,
}

/// In-memory store for pending OIDC token exchanges keyed by exchange code.
#[derive(Clone)]
pub struct OidcTokenExchangeStore {
    inner: Arc<Mutex<HashMap<String, PendingOidcTokenExchange>>>,
}

impl Default for OidcTokenExchangeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OidcTokenExchangeStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn insert(&self, code: String, exchange: PendingOidcTokenExchange) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .insert(code, exchange);
    }

    pub fn take(&self, code: &str) -> Option<PendingOidcTokenExchange> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .remove(code)
    }

    pub fn cleanup_expired(&self) {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(EXCHANGE_TTL_SECONDS);
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| {
                tracing::warn!("mutex poisoned, recovering inner data");
                poisoned.into_inner()
            })
            .retain(|_, ex| ex.created_at > cutoff);
    }
}
