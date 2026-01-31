use rand::Rng;
use rootcause::{Report, prelude::*};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{pending_device_flow, prelude::PendingDeviceFlow};
use uuid::Uuid;

use super::token::generate_secure_token;

/// TTL for device flow sessions (10 minutes).
const DEVICE_CODE_TTL_SECONDS: i64 = 600;

/// Minimum interval between poll requests (5 seconds).
pub const MIN_POLL_INTERVAL_SECONDS: i64 = 5;

/// Consonant alphabet for user codes (avoids vowels to prevent offensive words).
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

#[derive(Debug, Error)]
pub enum DeviceFlowError {
    #[error("device flow not found or expired")]
    NotFound,

    #[error("device flow already authorized")]
    AlreadyAuthorized,

    #[error("device flow polling too fast")]
    RateLimited,

    #[error("token generation failed: {0}")]
    TokenGeneration(String),

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, Report<DeviceFlowError>>;

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceFlowStatus {
    Pending,
    Authorized { user_id: Uuid },
    Expired,
}

/// Database-backed store for pending device authorization flows.
#[derive(Clone)]
pub struct DeviceFlowStore {
    db: DatabaseConnection,
}

impl DeviceFlowStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Create a new device flow session. Returns `(device_code, user_code)`.
    pub async fn create(&self, client_name: Option<String>) -> Result<(String, String)> {
        let device_code = generate_secure_token()
            .map_err(|e| report!(DeviceFlowError::TokenGeneration(e.to_string())))?;
        let user_code = generate_user_code();
        let raw_user_code = user_code.replace('-', "");

        let now = OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(DEVICE_CODE_TTL_SECONDS);

        let model = pending_device_flow::ActiveModel {
            device_code: Set(device_code.clone()),
            user_code: Set(raw_user_code),
            status: Set("pending".to_string()),
            user_id: Set(None),
            client_name: Set(client_name),
            created_at: Set(now),
            last_polled_at: Set(None),
            expires_at: Set(expires_at),
        };

        model
            .insert(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?;

        Ok((device_code, user_code))
    }

    /// Get the current status of a device flow by device code.
    pub async fn get_status(&self, device_code: &str) -> Result<DeviceFlowStatus> {
        let flow = PendingDeviceFlow::find_by_id(device_code)
            .one(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        let now = OffsetDateTime::now_utc();
        if flow.expires_at <= now {
            return Ok(DeviceFlowStatus::Expired);
        }

        match flow.status.as_str() {
            "authorized" => {
                let user_id = flow
                    .user_id
                    .ok_or_else(|| report!(DeviceFlowError::NotFound))?;
                Ok(DeviceFlowStatus::Authorized { user_id })
            }
            _ => Ok(DeviceFlowStatus::Pending),
        }
    }

    /// Check if polling is too fast (rate limiting).
    pub async fn is_rate_limited(&self, device_code: &str) -> Result<bool> {
        let flow = PendingDeviceFlow::find_by_id(device_code)
            .one(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        if let Some(last_polled) = flow.last_polled_at {
            let min_interval = time::Duration::seconds(MIN_POLL_INTERVAL_SECONDS);
            if OffsetDateTime::now_utc() - last_polled < min_interval {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Record a poll timestamp for rate limiting.
    pub async fn record_poll(&self, device_code: &str) -> Result<()> {
        let flow = PendingDeviceFlow::find_by_id(device_code)
            .one(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        let mut active: pending_device_flow::ActiveModel = flow.into();
        active.last_polled_at = Set(Some(OffsetDateTime::now_utc()));
        active
            .update(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?;

        Ok(())
    }

    /// Look up the client name for a device code.
    pub async fn get_client_name(&self, device_code: &str) -> Result<Option<String>> {
        let flow = PendingDeviceFlow::find_by_id(device_code)
            .one(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        Ok(flow.client_name)
    }

    /// Approve a device flow by user code, setting the authorized user.
    pub async fn approve(&self, user_code: &str, user_id: Uuid) -> Result<()> {
        let normalized = user_code.replace('-', "").to_uppercase();
        let now = OffsetDateTime::now_utc();

        // Find the flow by user code
        let flow = PendingDeviceFlow::find()
            .filter(pending_device_flow::Column::UserCode.eq(&normalized))
            .one(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        // Check expiry
        if flow.expires_at <= now {
            return Err(report!(DeviceFlowError::NotFound));
        }

        // Check already authorized
        if flow.status == "authorized" {
            return Err(report!(DeviceFlowError::AlreadyAuthorized));
        }

        // Atomic update: only update if still pending and not expired (HA-safe)
        let result = PendingDeviceFlow::update_many()
            .col_expr(
                pending_device_flow::Column::Status,
                sea_orm::sea_query::Expr::value("authorized"),
            )
            .col_expr(
                pending_device_flow::Column::UserId,
                sea_orm::sea_query::Expr::value(user_id),
            )
            .filter(pending_device_flow::Column::DeviceCode.eq(&flow.device_code))
            .filter(pending_device_flow::Column::Status.eq("pending"))
            .filter(pending_device_flow::Column::ExpiresAt.gt(now))
            .exec(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?;

        if result.rows_affected == 0 {
            // Another instance may have approved it, or it expired
            return Err(report!(DeviceFlowError::AlreadyAuthorized));
        }

        Ok(())
    }

    /// Consume a device flow (one-time use). Removes the flow from the database.
    pub async fn consume(&self, device_code: &str) -> Result<(Uuid, Option<String>)> {
        // Read the flow first
        let flow = PendingDeviceFlow::find_by_id(device_code)
            .one(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        if flow.status != "authorized" {
            return Err(report!(DeviceFlowError::NotFound));
        }

        let user_id = flow
            .user_id
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;
        let client_name = flow.client_name.clone();

        // Atomic delete: only delete if still authorized (HA-safe double-consume prevention)
        let result = PendingDeviceFlow::delete_many()
            .filter(pending_device_flow::Column::DeviceCode.eq(device_code))
            .filter(pending_device_flow::Column::Status.eq("authorized"))
            .exec(&self.db)
            .await
            .map_err(|e| report!(DeviceFlowError::Database(e)))?;

        if result.rows_affected == 0 {
            return Err(report!(DeviceFlowError::NotFound));
        }

        Ok((user_id, client_name))
    }

    /// Remove expired device flows.
    pub async fn cleanup_expired(&self) {
        let now = OffsetDateTime::now_utc();
        let result = PendingDeviceFlow::delete_many()
            .filter(pending_device_flow::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await;

        if let Err(e) = result {
            tracing::warn!("failed to clean up expired device flows: {e}");
        }
    }

    /// Test helper: backdate a flow's expiry to make it expired.
    #[cfg(test)]
    async fn expire_flow(&self, device_code: &str) {
        use sea_orm::ActiveValue::Unchanged;
        let expired_at =
            OffsetDateTime::now_utc() - time::Duration::seconds(DEVICE_CODE_TTL_SECONDS + 1);
        let model = pending_device_flow::ActiveModel {
            device_code: Unchanged(device_code.to_string()),
            expires_at: Set(expired_at),
            ..Default::default()
        };
        model.update(&self.db).await.expect("expire_flow update");
    }
}

/// Generate a user-friendly code: 8 uppercase consonants, formatted as XXXX-XXXX.
fn generate_user_code() -> String {
    let mut rng = rand::rng();
    let chars: Vec<u8> = (0..8)
        .map(|_| {
            let idx = rng.random_range(0..USER_CODE_ALPHABET.len());
            USER_CODE_ALPHABET[idx]
        })
        .collect();

    let first: String = chars[..4].iter().map(|&b| b as char).collect();
    let second: String = chars[4..].iter().map(|&b| b as char).collect();

    format!("{first}-{second}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Schema};

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");

        // Create table from entity
        let schema = Schema::new(db.get_database_backend());
        let stmt = schema.create_table_from_entity(PendingDeviceFlow);
        db.execute(&stmt).await.expect("create table");

        db
    }

    #[tokio::test]
    async fn test_create_flow() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, user_code) = store.create(Some("test-client".into())).await.unwrap();

        assert!(!device_code.is_empty());
        assert_eq!(user_code.len(), 9); // XXXX-XXXX
        assert_eq!(&user_code[4..5], "-");

        // All chars should be consonants
        for ch in user_code.replace('-', "").chars() {
            assert!(
                USER_CODE_ALPHABET.contains(&(ch as u8)),
                "unexpected char: {ch}"
            );
        }
    }

    #[tokio::test]
    async fn test_status_pending() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, _) = store.create(None).await.unwrap();

        let status = store.get_status(&device_code).await.unwrap();
        assert_eq!(status, DeviceFlowStatus::Pending);
    }

    #[tokio::test]
    async fn test_approve_and_status() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, user_code) = store.create(None).await.unwrap();
        let user_id = Uuid::now_v7();

        store.approve(&user_code, user_id).await.unwrap();

        let status = store.get_status(&device_code).await.unwrap();
        assert_eq!(status, DeviceFlowStatus::Authorized { user_id });
    }

    #[tokio::test]
    async fn test_approve_normalizes_code() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (_device_code, user_code) = store.create(None).await.unwrap();
        let user_id = Uuid::now_v7();

        // Approve with lowercase and hyphen
        let lower = user_code.to_lowercase();
        store.approve(&lower, user_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_approve_already_authorized() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (_device_code, user_code) = store.create(None).await.unwrap();
        let user_id = Uuid::now_v7();

        store.approve(&user_code, user_id).await.unwrap();

        let err = store.approve(&user_code, user_id).await.unwrap_err();
        assert!(matches!(
            err.current_context(),
            DeviceFlowError::AlreadyAuthorized
        ));
    }

    #[tokio::test]
    async fn test_consume_one_time_use() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, user_code) = store.create(Some("cli-host-2026".into())).await.unwrap();
        let user_id = Uuid::now_v7();

        store.approve(&user_code, user_id).await.unwrap();

        let (uid, client_name) = store.consume(&device_code).await.unwrap();
        assert_eq!(uid, user_id);
        assert_eq!(client_name.as_deref(), Some("cli-host-2026"));

        // Second consume should fail
        let err = store.consume(&device_code).await.unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));
    }

    #[tokio::test]
    async fn test_consume_pending_fails() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, _) = store.create(None).await.unwrap();

        let err = store.consume(&device_code).await.unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));
    }

    #[tokio::test]
    async fn test_not_found() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);

        let err = store.get_status("nonexistent").await.unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));

        let err = store
            .approve("NOPE-CODE", Uuid::now_v7())
            .await
            .unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, _) = store.create(None).await.unwrap();

        // First poll: not rate limited
        assert!(!store.is_rate_limited(&device_code).await.unwrap());

        // Record poll
        store.record_poll(&device_code).await.unwrap();

        // Immediately poll again: should be rate limited
        assert!(store.is_rate_limited(&device_code).await.unwrap());
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);

        // Create a flow and backdate it
        let (device_code, _user_code) = store.create(None).await.unwrap();
        store.expire_flow(&device_code).await;

        store.cleanup_expired().await;

        // Flow should be gone
        let err = store.get_status(&device_code).await.unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));
    }

    #[tokio::test]
    async fn test_expired_flow_returns_expired_status() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, _) = store.create(None).await.unwrap();

        // Backdate the flow
        store.expire_flow(&device_code).await;

        let status = store.get_status(&device_code).await.unwrap();
        assert_eq!(status, DeviceFlowStatus::Expired);
    }

    #[test]
    fn test_user_code_format() {
        // Generate many codes and verify format
        for _ in 0..100 {
            let code = generate_user_code();
            assert_eq!(code.len(), 9);
            assert_eq!(code.as_bytes()[4], b'-');
            for ch in code.replace('-', "").chars() {
                assert!(USER_CODE_ALPHABET.contains(&(ch as u8)));
            }
        }
    }
}
