use openidconnect::{Nonce, PkceCodeVerifier};
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    pending_account_link, pending_oidc_flow, pending_oidc_registration,
    pending_oidc_token_exchange,
    prelude::{
        PendingAccountLink, PendingOidcFlow, PendingOidcRegistration, PendingOidcTokenExchange,
    },
};
use uptrakit_shared_macros::impl_report_conversion;

const TTL_SECONDS: i64 = 600; // 10 minutes
const EXCHANGE_TTL_SECONDS: i64 = 60; // 60 seconds

#[derive(Debug, Error)]
pub enum OidcStoreError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<OidcStoreError>>;

impl_report_conversion! {
    sea_orm::DbErr     => OidcStoreError::Database,
    serde_json::Error  => OidcStoreError::Serialization,
}

/// Pending OIDC authorization flow data returned by `take()`.
pub struct PendingOidcFlowData {
    pub provider_id: uuid::Uuid,
    pub pkce_verifier: PkceCodeVerifier,
    pub nonce: Nonce,
}

/// Database-backed store for pending OIDC flows keyed by `state` parameter.
#[derive(Clone)]
pub struct OidcFlowStore {
    db: DatabaseConnection,
}

impl OidcFlowStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn insert(
        &self,
        state: String,
        provider_id: uuid::Uuid,
        pkce_verifier: &PkceCodeVerifier,
        nonce: &Nonce,
    ) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(TTL_SECONDS);

        let model = pending_oidc_flow::ActiveModel {
            csrf_state: Set(state),
            provider_id: Set(provider_id),
            pkce_verifier: Set(pkce_verifier.secret().clone()),
            nonce: Set(nonce.secret().clone()),
            created_at: Set(now),
            expires_at: Set(expires_at),
        };

        model.insert(&self.db).await.context_to()?;

        Ok(())
    }

    pub async fn take(&self, state: &str) -> Result<Option<PendingOidcFlowData>> {
        let now = OffsetDateTime::now_utc();

        let flow = match PendingOidcFlow::find_by_id(state)
            .one(&self.db)
            .await
            .context_to()?
        {
            Some(f) => f,
            None => return Ok(None),
        };

        // Atomic delete: only delete if not expired (HA-safe)
        let result = PendingOidcFlow::delete_many()
            .filter(pending_oidc_flow::Column::CsrfState.eq(state))
            .filter(pending_oidc_flow::Column::ExpiresAt.gt(now))
            .exec(&self.db)
            .await
            .context_to()?;

        if result.rows_affected == 0 {
            return Ok(None);
        }

        Ok(Some(PendingOidcFlowData {
            provider_id: flow.provider_id,
            pkce_verifier: PkceCodeVerifier::new(flow.pkce_verifier),
            nonce: Nonce::new(flow.nonce),
        }))
    }

    pub async fn cleanup_expired(&self) {
        let now = OffsetDateTime::now_utc();
        let result = PendingOidcFlow::delete_many()
            .filter(pending_oidc_flow::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await;

        if let Err(e) = result {
            tracing::warn!("failed to clean up expired OIDC flows: {e}");
        }
    }
}

/// Parameters for inserting a pending account link.
pub struct PendingAccountLinkParams {
    pub token: String,
    pub provider_id: uuid::Uuid,
    pub oidc_subject: String,
    pub email: String,
    pub user_id: uuid::Uuid,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub mapped_roles: Vec<String>,
    pub existing_link_provider_id: Option<uuid::Uuid>,
}

/// Pending account link data returned by `take()`.
pub struct PendingAccountLinkData {
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
}

/// Database-backed store for pending account links keyed by a random token.
#[derive(Clone)]
pub struct AccountLinkStore {
    db: DatabaseConnection,
}

impl AccountLinkStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn insert(&self, params: PendingAccountLinkParams) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(TTL_SECONDS);

        let roles_json = serde_json::to_value(&params.mapped_roles).context_to()?;

        let model = pending_account_link::ActiveModel {
            link_token: Set(params.token),
            provider_id: Set(params.provider_id),
            oidc_subject: Set(params.oidc_subject),
            email: Set(params.email),
            user_id: Set(params.user_id),
            first_name: Set(params.first_name),
            last_name: Set(params.last_name),
            mapped_roles: Set(roles_json),
            existing_link_provider_id: Set(params.existing_link_provider_id),
            created_at: Set(now),
            expires_at: Set(expires_at),
        };

        model.insert(&self.db).await.context_to()?;

        Ok(())
    }

    pub async fn take(&self, token: &str) -> Result<Option<PendingAccountLinkData>> {
        let now = OffsetDateTime::now_utc();

        let link = match PendingAccountLink::find_by_id(token)
            .one(&self.db)
            .await
            .context_to()?
        {
            Some(l) => l,
            None => return Ok(None),
        };

        // Atomic delete: only delete if not expired (HA-safe)
        let result = PendingAccountLink::delete_many()
            .filter(pending_account_link::Column::LinkToken.eq(token))
            .filter(pending_account_link::Column::ExpiresAt.gt(now))
            .exec(&self.db)
            .await
            .context_to()?;

        if result.rows_affected == 0 {
            return Ok(None);
        }

        let mapped_roles: Vec<String> = serde_json::from_value(link.mapped_roles).context_to()?;

        Ok(Some(PendingAccountLinkData {
            provider_id: link.provider_id,
            oidc_subject: link.oidc_subject,
            email: link.email,
            user_id: link.user_id,
            first_name: link.first_name,
            last_name: link.last_name,
            mapped_roles,
            existing_link_provider_id: link.existing_link_provider_id,
        }))
    }

    pub async fn cleanup_expired(&self) {
        let now = OffsetDateTime::now_utc();
        let result = PendingAccountLink::delete_many()
            .filter(pending_account_link::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await;

        if let Err(e) = result {
            tracing::warn!("failed to clean up expired account links: {e}");
        }
    }
}

/// Pending OIDC token exchange data returned by `take()`.
///
/// Only stores `user_id` and `provider_id` — actual tokens are created
/// on-demand when the exchange code is consumed by the frontend.
pub struct PendingOidcTokenExchangeData {
    pub user_id: uuid::Uuid,
    pub provider_id: uuid::Uuid,
}

/// Database-backed store for pending OIDC token exchanges keyed by exchange code.
#[derive(Clone)]
pub struct OidcTokenExchangeStore {
    db: DatabaseConnection,
}

impl OidcTokenExchangeStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn insert(
        &self,
        code: String,
        user_id: uuid::Uuid,
        provider_id: uuid::Uuid,
    ) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(EXCHANGE_TTL_SECONDS);

        let model = pending_oidc_token_exchange::ActiveModel {
            exchange_code: Set(code),
            user_id: Set(user_id),
            provider_id: Set(provider_id),
            created_at: Set(now),
            expires_at: Set(expires_at),
        };

        model.insert(&self.db).await.context_to()?;

        Ok(())
    }

    pub async fn take(&self, code: &str) -> Result<Option<PendingOidcTokenExchangeData>> {
        let now = OffsetDateTime::now_utc();

        let exchange = match PendingOidcTokenExchange::find_by_id(code)
            .one(&self.db)
            .await
            .context_to()?
        {
            Some(e) => e,
            None => return Ok(None),
        };

        // Atomic delete: only delete if not expired (HA-safe)
        let result = PendingOidcTokenExchange::delete_many()
            .filter(pending_oidc_token_exchange::Column::ExchangeCode.eq(code))
            .filter(pending_oidc_token_exchange::Column::ExpiresAt.gt(now))
            .exec(&self.db)
            .await
            .context_to()?;

        if result.rows_affected == 0 {
            return Ok(None);
        }

        Ok(Some(PendingOidcTokenExchangeData {
            user_id: exchange.user_id,
            provider_id: exchange.provider_id,
        }))
    }

    pub async fn cleanup_expired(&self) {
        let now = OffsetDateTime::now_utc();
        let result = PendingOidcTokenExchange::delete_many()
            .filter(pending_oidc_token_exchange::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await;

        if let Err(e) = result {
            tracing::warn!("failed to clean up expired OIDC token exchanges: {e}");
        }
    }
}

/// Parameters for inserting a pending OIDC registration.
pub struct PendingOidcRegistrationParams {
    pub registration_code: String,
    pub provider_id: uuid::Uuid,
    pub oidc_subject: String,
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub mapped_roles: Vec<String>,
}

/// Pending OIDC registration data returned by `take()`.
pub struct PendingOidcRegistrationData {
    pub provider_id: uuid::Uuid,
    pub oidc_subject: String,
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    /// Pre-mapped local role names to assign after registration.
    pub mapped_roles: Vec<String>,
}

/// Database-backed store for pending OIDC registrations keyed by registration code.
#[derive(Clone)]
pub struct OidcRegistrationStore {
    db: DatabaseConnection,
}

impl OidcRegistrationStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn insert(&self, params: PendingOidcRegistrationParams) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(TTL_SECONDS);

        let roles_json = serde_json::to_value(&params.mapped_roles).context_to()?;

        let model = pending_oidc_registration::ActiveModel {
            registration_code: Set(params.registration_code),
            provider_id: Set(params.provider_id),
            oidc_subject: Set(params.oidc_subject),
            email: Set(params.email),
            first_name: Set(params.first_name),
            last_name: Set(params.last_name),
            mapped_roles: Set(roles_json),
            created_at: Set(now),
            expires_at: Set(expires_at),
        };

        model.insert(&self.db).await.context_to()?;

        Ok(())
    }

    /// Non-destructive read: returns the pending registration if it exists and is
    /// not expired, without removing it from the store. Use this to validate
    /// preconditions (e.g. registration token) before consuming with [`take()`].
    pub async fn get(&self, code: &str) -> Result<Option<PendingOidcRegistrationData>> {
        let now = OffsetDateTime::now_utc();

        let reg = match PendingOidcRegistration::find_by_id(code)
            .filter(pending_oidc_registration::Column::ExpiresAt.gt(now))
            .one(&self.db)
            .await
            .context_to()?
        {
            Some(r) => r,
            None => return Ok(None),
        };

        let mapped_roles: Vec<String> = serde_json::from_value(reg.mapped_roles).context_to()?;

        Ok(Some(PendingOidcRegistrationData {
            provider_id: reg.provider_id,
            oidc_subject: reg.oidc_subject,
            email: reg.email,
            first_name: reg.first_name,
            last_name: reg.last_name,
            mapped_roles,
        }))
    }

    pub async fn take(&self, code: &str) -> Result<Option<PendingOidcRegistrationData>> {
        let now = OffsetDateTime::now_utc();

        let reg = match PendingOidcRegistration::find_by_id(code)
            .one(&self.db)
            .await
            .context_to()?
        {
            Some(r) => r,
            None => return Ok(None),
        };

        // Atomic delete: only delete if not expired (HA-safe)
        let result = PendingOidcRegistration::delete_many()
            .filter(pending_oidc_registration::Column::RegistrationCode.eq(code))
            .filter(pending_oidc_registration::Column::ExpiresAt.gt(now))
            .exec(&self.db)
            .await
            .context_to()?;

        if result.rows_affected == 0 {
            return Ok(None);
        }

        let mapped_roles: Vec<String> = serde_json::from_value(reg.mapped_roles).context_to()?;

        Ok(Some(PendingOidcRegistrationData {
            provider_id: reg.provider_id,
            oidc_subject: reg.oidc_subject,
            email: reg.email,
            first_name: reg.first_name,
            last_name: reg.last_name,
            mapped_roles,
        }))
    }

    pub async fn cleanup_expired(&self) {
        let now = OffsetDateTime::now_utc();
        let result = PendingOidcRegistration::delete_many()
            .filter(pending_oidc_registration::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await;

        if let Err(e) = result {
            tracing::warn!("failed to clean up expired OIDC registrations: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, Schema};

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        let schema = Schema::new(db.get_database_backend());

        // Create all 3 tables
        let stmt = schema.create_table_from_entity(PendingOidcFlow);
        db.execute(&stmt).await.expect("create oidc_flow table");
        let stmt = schema.create_table_from_entity(PendingAccountLink);
        db.execute(&stmt).await.expect("create account_link table");
        let stmt = schema.create_table_from_entity(PendingOidcTokenExchange);
        db.execute(&stmt)
            .await
            .expect("create token_exchange table");
        let stmt = schema.create_table_from_entity(PendingOidcRegistration);
        db.execute(&stmt)
            .await
            .expect("create oidc_registration table");

        db
    }

    #[tokio::test]
    async fn test_oidc_flow_insert_and_take() {
        let db = test_db().await;
        let store = OidcFlowStore::new(db);

        let provider_id = uuid::Uuid::now_v7();
        let pkce = PkceCodeVerifier::new("test-verifier".to_string());
        let nonce = Nonce::new("test-nonce".to_string());

        store
            .insert("csrf-state-1".to_string(), provider_id, &pkce, &nonce)
            .await
            .unwrap();

        let flow = store.take("csrf-state-1").await.unwrap();
        assert!(flow.is_some());
        let flow = flow.unwrap();
        assert_eq!(flow.provider_id, provider_id);
        assert_eq!(flow.pkce_verifier.secret(), "test-verifier");
        assert_eq!(flow.nonce.secret(), "test-nonce");

        // Second take should return None
        let flow = store.take("csrf-state-1").await.unwrap();
        assert!(flow.is_none());
    }

    #[tokio::test]
    async fn test_oidc_flow_take_nonexistent() {
        let db = test_db().await;
        let store = OidcFlowStore::new(db);

        let flow = store.take("nonexistent").await.unwrap();
        assert!(flow.is_none());
    }

    #[tokio::test]
    async fn test_account_link_insert_and_take() {
        let db = test_db().await;
        let store = AccountLinkStore::new(db);

        let provider_id = uuid::Uuid::now_v7();
        let user_id = uuid::Uuid::now_v7();

        store
            .insert(PendingAccountLinkParams {
                token: "link-token-1".to_string(),
                provider_id,
                oidc_subject: "subject-123".to_string(),
                email: "test@example.com".to_string(),
                user_id,
                first_name: Some("Test".to_string()),
                last_name: Some("User".to_string()),
                mapped_roles: vec!["admin".to_string()],
                existing_link_provider_id: None,
            })
            .await
            .unwrap();

        let link = store.take("link-token-1").await.unwrap();
        assert!(link.is_some());
        let link = link.unwrap();
        assert_eq!(link.provider_id, provider_id);
        assert_eq!(link.user_id, user_id);
        assert_eq!(link.oidc_subject, "subject-123");
        assert_eq!(link.email, "test@example.com");
        assert_eq!(link.first_name.as_deref(), Some("Test"));
        assert_eq!(link.last_name.as_deref(), Some("User"));
        assert_eq!(link.mapped_roles, vec!["admin".to_string()]);
        assert!(link.existing_link_provider_id.is_none());

        // Second take should return None
        let link = store.take("link-token-1").await.unwrap();
        assert!(link.is_none());
    }

    #[tokio::test]
    async fn test_token_exchange_insert_and_take() {
        let db = test_db().await;
        let store = OidcTokenExchangeStore::new(db);

        let user_id = uuid::Uuid::now_v7();
        let provider_id = uuid::Uuid::now_v7();

        store
            .insert("exchange-code-1".to_string(), user_id, provider_id)
            .await
            .unwrap();

        let exchange = store.take("exchange-code-1").await.unwrap();
        assert!(exchange.is_some());
        let exchange = exchange.unwrap();
        assert_eq!(exchange.user_id, user_id);
        assert_eq!(exchange.provider_id, provider_id);

        // Second take should return None
        let exchange = store.take("exchange-code-1").await.unwrap();
        assert!(exchange.is_none());
    }

    #[tokio::test]
    async fn test_token_exchange_take_nonexistent() {
        let db = test_db().await;
        let store = OidcTokenExchangeStore::new(db);

        let exchange = store.take("nonexistent").await.unwrap();
        assert!(exchange.is_none());
    }

    #[tokio::test]
    async fn test_oidc_flow_cleanup() {
        let db = test_db().await;
        let store = OidcFlowStore::new(db.clone());

        let pkce = PkceCodeVerifier::new("verifier".to_string());
        let nonce = Nonce::new("nonce".to_string());

        store
            .insert(
                "state-to-expire".to_string(),
                uuid::Uuid::now_v7(),
                &pkce,
                &nonce,
            )
            .await
            .unwrap();

        // Backdate the flow to make it expired
        db.execute_unprepared(
            "UPDATE pending_oidc_flows SET expires_at = datetime('now', '-1 hour') WHERE csrf_state = 'state-to-expire'"
        ).await.unwrap();

        store.cleanup_expired().await;

        // Flow should be gone
        let flow = store.take("state-to-expire").await.unwrap();
        assert!(flow.is_none());
    }

    #[tokio::test]
    async fn test_account_link_cleanup() {
        let db = test_db().await;
        let store = AccountLinkStore::new(db.clone());

        store
            .insert(PendingAccountLinkParams {
                token: "token-to-expire".to_string(),
                provider_id: uuid::Uuid::now_v7(),
                oidc_subject: "sub".to_string(),
                email: "e@x.com".to_string(),
                user_id: uuid::Uuid::now_v7(),
                first_name: None,
                last_name: None,
                mapped_roles: vec![],
                existing_link_provider_id: None,
            })
            .await
            .unwrap();

        // Backdate the link
        db.execute_unprepared(
            "UPDATE pending_account_links SET expires_at = datetime('now', '-1 hour') WHERE link_token = 'token-to-expire'"
        ).await.unwrap();

        store.cleanup_expired().await;

        let link = store.take("token-to-expire").await.unwrap();
        assert!(link.is_none());
    }

    #[tokio::test]
    async fn test_token_exchange_cleanup() {
        let db = test_db().await;
        let store = OidcTokenExchangeStore::new(db.clone());

        store
            .insert(
                "code-to-expire".to_string(),
                uuid::Uuid::now_v7(),
                uuid::Uuid::now_v7(),
            )
            .await
            .unwrap();

        // Backdate the exchange
        db.execute_unprepared(
            "UPDATE pending_oidc_token_exchanges SET expires_at = datetime('now', '-1 hour') WHERE exchange_code = 'code-to-expire'"
        ).await.unwrap();

        store.cleanup_expired().await;

        let exchange = store.take("code-to-expire").await.unwrap();
        assert!(exchange.is_none());
    }

    #[tokio::test]
    async fn test_oidc_registration_insert_and_take() {
        let db = test_db().await;
        let store = OidcRegistrationStore::new(db);

        let provider_id = uuid::Uuid::now_v7();

        store
            .insert(PendingOidcRegistrationParams {
                registration_code: "reg-code-1".to_string(),
                provider_id,
                oidc_subject: "subject-456".to_string(),
                email: "newuser@example.com".to_string(),
                first_name: Some("New".to_string()),
                last_name: Some("User".to_string()),
                mapped_roles: vec!["viewer".to_string()],
            })
            .await
            .unwrap();

        let reg = store.take("reg-code-1").await.unwrap();
        assert!(reg.is_some());
        let reg = reg.unwrap();
        assert_eq!(reg.provider_id, provider_id);
        assert_eq!(reg.oidc_subject, "subject-456");
        assert_eq!(reg.email, "newuser@example.com");
        assert_eq!(reg.first_name.as_deref(), Some("New"));
        assert_eq!(reg.last_name.as_deref(), Some("User"));
        assert_eq!(reg.mapped_roles, vec!["viewer".to_string()]);

        // Second take should return None
        let reg = store.take("reg-code-1").await.unwrap();
        assert!(reg.is_none());
    }

    #[tokio::test]
    async fn test_oidc_registration_take_nonexistent() {
        let db = test_db().await;
        let store = OidcRegistrationStore::new(db);

        let reg = store.take("nonexistent").await.unwrap();
        assert!(reg.is_none());
    }

    #[tokio::test]
    async fn test_oidc_registration_get_returns_data_without_consuming() {
        let db = test_db().await;
        let store = OidcRegistrationStore::new(db);

        let provider_id = uuid::Uuid::now_v7();

        store
            .insert(PendingOidcRegistrationParams {
                registration_code: "reg-get-1".to_string(),
                provider_id,
                oidc_subject: "subject-get".to_string(),
                email: "get@example.com".to_string(),
                first_name: Some("Get".to_string()),
                last_name: Some("Test".to_string()),
                mapped_roles: vec!["viewer".to_string()],
            })
            .await
            .unwrap();

        // First get returns data
        let reg = store.get("reg-get-1").await.unwrap();
        assert!(reg.is_some());
        let reg = reg.unwrap();
        assert_eq!(reg.provider_id, provider_id);
        assert_eq!(reg.email, "get@example.com");

        // Second get still returns data (not consumed)
        let reg = store.get("reg-get-1").await.unwrap();
        assert!(reg.is_some());

        // take() still works after get()
        let reg = store.take("reg-get-1").await.unwrap();
        assert!(reg.is_some());

        // Now it's consumed
        let reg = store.get("reg-get-1").await.unwrap();
        assert!(reg.is_none());
    }

    #[tokio::test]
    async fn test_oidc_registration_get_nonexistent() {
        let db = test_db().await;
        let store = OidcRegistrationStore::new(db);

        let reg = store.get("nonexistent").await.unwrap();
        assert!(reg.is_none());
    }

    #[tokio::test]
    async fn test_oidc_registration_get_expired() {
        let db = test_db().await;
        let store = OidcRegistrationStore::new(db.clone());

        store
            .insert(PendingOidcRegistrationParams {
                registration_code: "reg-get-expired".to_string(),
                provider_id: uuid::Uuid::now_v7(),
                oidc_subject: "sub".to_string(),
                email: "e@x.com".to_string(),
                first_name: None,
                last_name: None,
                mapped_roles: vec![],
            })
            .await
            .unwrap();

        // Backdate to make it expired
        db.execute_unprepared(
            "UPDATE pending_oidc_registrations SET expires_at = datetime('now', '-1 hour') WHERE registration_code = 'reg-get-expired'"
        ).await.unwrap();

        let reg = store.get("reg-get-expired").await.unwrap();
        assert!(reg.is_none());
    }

    #[tokio::test]
    async fn test_oidc_registration_retry_after_failed_validation() {
        let db = test_db().await;
        let store = OidcRegistrationStore::new(db);

        let provider_id = uuid::Uuid::now_v7();

        store
            .insert(PendingOidcRegistrationParams {
                registration_code: "reg-retry-1".to_string(),
                provider_id,
                oidc_subject: "subject-retry".to_string(),
                email: "retry@example.com".to_string(),
                first_name: Some("Retry".to_string()),
                last_name: None,
                mapped_roles: vec![],
            })
            .await
            .unwrap();

        // Simulate the fixed handler flow: get() to peek, then validation fails,
        // entry should still be available for retry
        let reg = store.get("reg-retry-1").await.unwrap();
        assert!(reg.is_some());

        // "Validation failed" — do NOT call take(), entry stays in store

        // User retries with correct token — get() still works
        let reg = store.get("reg-retry-1").await.unwrap();
        assert!(reg.is_some());

        // "Validation passed" — now consume with take()
        let reg = store.take("reg-retry-1").await.unwrap();
        assert!(reg.is_some());
        let reg = reg.unwrap();
        assert_eq!(reg.provider_id, provider_id);
        assert_eq!(reg.email, "retry@example.com");

        // Entry is consumed — no more retries
        let reg = store.get("reg-retry-1").await.unwrap();
        assert!(reg.is_none());
    }

    #[tokio::test]
    async fn test_oidc_registration_cleanup() {
        let db = test_db().await;
        let store = OidcRegistrationStore::new(db.clone());

        store
            .insert(PendingOidcRegistrationParams {
                registration_code: "reg-to-expire".to_string(),
                provider_id: uuid::Uuid::now_v7(),
                oidc_subject: "sub".to_string(),
                email: "e@x.com".to_string(),
                first_name: None,
                last_name: None,
                mapped_roles: vec![],
            })
            .await
            .unwrap();

        // Backdate the registration
        db.execute_unprepared(
            "UPDATE pending_oidc_registrations SET expires_at = datetime('now', '-1 hour') WHERE registration_code = 'reg-to-expire'"
        ).await.unwrap();

        store.cleanup_expired().await;

        let reg = store.take("reg-to-expire").await.unwrap();
        assert!(reg.is_none());
    }
}
