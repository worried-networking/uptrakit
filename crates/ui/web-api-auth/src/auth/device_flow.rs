use rand::Rng;
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::api_token;
use uptrakit_shared_db::entity::pending_device_flow;
use uptrakit_shared_db::entity::prelude::PendingDeviceFlow;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_shared_types::{DeviceAuthStatus, SecretString};
use uptrakit_web_api_types::oauth::OAuthErrorCode;
use uuid::Uuid;

use super::token::{generate_secure_token, generate_uuid, hash_token};

/// TTL for device flow sessions (10 minutes).
const DEVICE_CODE_TTL_SECONDS: i64 = 600;

/// Consonant alphabet for user codes (avoids vowels to prevent offensive words).
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

/// Hardcoded OAuth public-client identifier for the CLI. Future migration
/// (Seam 3 in the spec): replace this constant with a lookup against an
/// `oauth_clients` allowlist table.
pub const CLIENT_ID: &str = "uptrakit-cli";

/// Default polling interval (seconds) returned to clients on flow creation.
pub const POLL_INTERVAL_SECONDS: i32 = 5;

/// How many seconds to add to `interval` each time a `slow_down` is returned
/// (per RFC 8628 §3.5 client-side bump rule).
pub const POLL_INTERVAL_BUMP_SECONDS: i32 = 5;

#[derive(Debug, Error)]
pub enum DeviceFlowError {
    #[error("device flow not found or expired")]
    NotFound,

    #[error("device flow already authorized")]
    AlreadyAuthorized,

    #[error("token generation failed: {0}")]
    TokenGeneration(String),

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, Report<DeviceFlowError>>;

impl_report_conversion!(sea_orm::DbErr => DeviceFlowError::Database);

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceFlowStatus {
    Pending,
    Authorized { user_id: Uuid },
    Expired,
}

/// Outcome of a single `poll()` call. The route layer maps these onto
/// RFC 8628 §3.5 wire codes.
///
/// `#[non_exhaustive]` because this type crosses the `web-api-auth` →
/// `web-api` crate boundary and matches against the project's standard for
/// extensible public enums. External match sites carry a wildcard arm with
/// `tracing::warn!` per `docs/development/coding-standards.md`.
#[non_exhaustive]
#[derive(Debug)]
pub enum PollOutcome {
    /// Flow is approved; token has been minted.
    Authorized {
        token: SecretString,
        token_name: String,
    },
    /// Flow is still pending; client should poll again after `interval` seconds.
    Pending,
    /// Client polled too fast; bumped `interval` is returned to it.
    SlowDown { bumped_interval: i32 },
    /// Operator denied this flow.
    Denied,
    /// Flow has expired.
    Expired,
    /// Device code is unknown (route layer collapses this into `expired_token`).
    Unknown,
    /// Device code is malformed (route layer maps to `invalid_grant`).
    MalformedDeviceCode,
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
    ///
    /// # Errors
    ///
    /// Returns [`DeviceFlowError::Database`] on a DB insert error.
    pub async fn create(
        &self,
        client_name: Option<String>,
        scope: Option<String>,
    ) -> Result<(String, String)> {
        let id = generate_uuid();
        let device_code = generate_secure_token()
            .map_err(|e| report!(DeviceFlowError::TokenGeneration(e.to_string())))?;
        let device_code_hash = hash_token(&device_code);
        let user_code = generate_user_code();
        let raw_user_code = user_code.replace('-', "");

        let now = OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(DEVICE_CODE_TTL_SECONDS);

        let model = pending_device_flow::ActiveModel {
            id: Set(id),
            device_code_hash: Set(device_code_hash),
            user_code: Set(raw_user_code),
            status: Set(DeviceAuthStatus::Pending),
            user_id: Set(None),
            denied_by: Set(None),
            client_name: Set(client_name),
            scope: Set(scope),
            interval: Set(POLL_INTERVAL_SECONDS),
            last_polled_at: Set(None),
            created_at: Set(now),
            expires_at: Set(expires_at),
        };

        model.insert(&self.db).await.context_to()?;

        Ok((device_code, user_code))
    }

    /// Poll a device flow. RFC 8628 §3.4–§3.5.
    ///
    /// All branches run inside a single `BEGIN IMMEDIATE` SQLite transaction
    /// (per CLAUDE.md "SQLite Transaction Rules": read-then-write must use
    /// Immediate to avoid `SQLITE_BUSY_SNAPSHOT`). On Postgres this is a no-op
    /// per SeaORM.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceFlowError::Database`] on a DB error, or
    /// [`DeviceFlowError::NotFound`] if the authorized flow row has no
    /// `user_id` (which would indicate data corruption).
    pub async fn poll(&self, device_code: &str, now: OffsetDateTime) -> Result<PollOutcome> {
        if device_code.is_empty() {
            return Ok(PollOutcome::MalformedDeviceCode);
        }
        let hash = hash_token(device_code);

        let txn = self
            .db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await
            .context_to()?;

        let flow_opt = PendingDeviceFlow::find()
            .filter(pending_device_flow::Column::DeviceCodeHash.eq(&hash))
            .one(&txn)
            .await
            .context_to()?;

        let Some(flow) = flow_opt else {
            txn.commit().await.context_to()?;
            return Ok(PollOutcome::Unknown);
        };

        if flow.expires_at <= now {
            txn.commit().await.context_to()?;
            return Ok(PollOutcome::Expired);
        }

        match flow.status {
            DeviceAuthStatus::Authorized => {
                let user_id = flow
                    .user_id
                    .ok_or_else(|| report!(DeviceFlowError::NotFound))?;
                let token_name = flow.client_name.clone().unwrap_or_else(|| "cli".into());

                // Atomic conditional delete; matches the old `consume` HA-safe pattern.
                let result = PendingDeviceFlow::delete_many()
                    .filter(pending_device_flow::Column::Id.eq(flow.id))
                    .filter(
                        pending_device_flow::Column::Status
                            .eq(DeviceAuthStatus::Authorized.as_str()),
                    )
                    .exec(&txn)
                    .await
                    .context_to()?;
                if result.rows_affected == 0 {
                    txn.commit().await.context_to()?;
                    return Ok(PollOutcome::Unknown);
                }

                // Mint the API token inside the same txn so the consume-delete
                // and the api_token insert commit atomically.
                let (token_id, token) = issue_access_token(&txn, user_id, &token_name).await?;
                // Seam 2: scope enforcement. Today a no-op; future migration
                // maps the scope string to a Permission subset on the token.
                apply_scope_to_token(token_id, flow.scope.as_deref());
                txn.commit().await.context_to()?;
                Ok(PollOutcome::Authorized { token, token_name })
            }
            DeviceAuthStatus::Denied => {
                txn.commit().await.context_to()?;
                Ok(PollOutcome::Denied)
            }
            DeviceAuthStatus::Expired => {
                txn.commit().await.context_to()?;
                Ok(PollOutcome::Expired)
            }
            DeviceAuthStatus::Pending => {
                let interval = flow.interval;
                let too_fast = matches!(
                    flow.last_polled_at,
                    Some(prev) if (now - prev).whole_seconds() < interval as i64
                );

                if too_fast {
                    let bumped = interval.saturating_add(POLL_INTERVAL_BUMP_SECONDS);
                    PendingDeviceFlow::update_many()
                        .col_expr(
                            pending_device_flow::Column::Interval,
                            sea_orm::sea_query::Expr::value(bumped),
                        )
                        .col_expr(
                            pending_device_flow::Column::LastPolledAt,
                            sea_orm::sea_query::Expr::value(now),
                        )
                        .filter(pending_device_flow::Column::Id.eq(flow.id))
                        .exec(&txn)
                        .await
                        .context_to()?;
                    txn.commit().await.context_to()?;
                    Ok(PollOutcome::SlowDown {
                        bumped_interval: bumped,
                    })
                } else {
                    PendingDeviceFlow::update_many()
                        .col_expr(
                            pending_device_flow::Column::LastPolledAt,
                            sea_orm::sea_query::Expr::value(now),
                        )
                        .filter(pending_device_flow::Column::Id.eq(flow.id))
                        .exec(&txn)
                        .await
                        .context_to()?;
                    txn.commit().await.context_to()?;
                    Ok(PollOutcome::Pending)
                }
            }
            _ => {
                tracing::warn!(status = ?flow.status, "device flow has unexpected status");
                txn.commit().await.context_to()?;
                Ok(PollOutcome::Unknown)
            }
        }
    }

    /// Deny a pending device flow. RFC 8628 access_denied path.
    ///
    /// Read-then-write under `BEGIN IMMEDIATE` so the row read and the atomic
    /// CAS commit on the same SQLite connection.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceFlowError::NotFound`] when the flow does not exist or
    /// has expired. Returns [`DeviceFlowError::AlreadyAuthorized`] when the
    /// flow is no longer in `Pending` state.
    pub async fn deny(&self, user_code: &str, denied_by: Uuid) -> Result<()> {
        let normalized = user_code.replace('-', "").to_uppercase();
        let now = OffsetDateTime::now_utc();

        let txn = self
            .db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await
            .context_to()?;

        let flow_opt = PendingDeviceFlow::find()
            .filter(pending_device_flow::Column::UserCode.eq(&normalized))
            .one(&txn)
            .await
            .context_to()?;

        let Some(flow) = flow_opt else {
            txn.commit().await.context_to()?;
            bail!(DeviceFlowError::NotFound);
        };

        if flow.expires_at <= now {
            txn.commit().await.context_to()?;
            bail!(DeviceFlowError::NotFound);
        }
        if flow.status != DeviceAuthStatus::Pending {
            txn.commit().await.context_to()?;
            bail!(DeviceFlowError::AlreadyAuthorized);
        }

        let result = PendingDeviceFlow::update_many()
            .col_expr(
                pending_device_flow::Column::Status,
                sea_orm::sea_query::Expr::value(DeviceAuthStatus::Denied.as_str()),
            )
            .col_expr(
                pending_device_flow::Column::DeniedBy,
                sea_orm::sea_query::Expr::value(denied_by),
            )
            .filter(pending_device_flow::Column::Id.eq(flow.id))
            .filter(pending_device_flow::Column::Status.eq(DeviceAuthStatus::Pending.as_str()))
            .filter(pending_device_flow::Column::ExpiresAt.gt(now))
            .exec(&txn)
            .await
            .context_to()?;
        if result.rows_affected == 0 {
            txn.commit().await.context_to()?;
            bail!(DeviceFlowError::AlreadyAuthorized);
        }
        txn.commit().await.context_to()?;
        Ok(())
    }

    /// Look up the client name for a device code.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceFlowError::NotFound`] if no flow matches the device code,
    /// or [`DeviceFlowError::Database`] on a DB error.
    pub async fn get_client_name(&self, device_code: &str) -> Result<Option<String>> {
        let hash = hash_token(device_code);
        let flow = PendingDeviceFlow::find()
            .filter(pending_device_flow::Column::DeviceCodeHash.eq(&hash))
            .one(&self.db)
            .await
            .context_to()?
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        Ok(flow.client_name)
    }

    /// Approve a device flow by user code, setting the authorized user.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceFlowError::NotFound`] if the flow is missing or expired,
    /// [`DeviceFlowError::AlreadyAuthorized`] if it was already approved or the
    /// atomic update found 0 rows, or [`DeviceFlowError::Database`] on a DB error.
    pub async fn approve(&self, user_code: &str, user_id: Uuid) -> Result<()> {
        let normalized = user_code.replace('-', "").to_uppercase();
        let now = OffsetDateTime::now_utc();

        let txn = self
            .db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await
            .context_to()?;

        // Find the flow by user code
        let flow = PendingDeviceFlow::find()
            .filter(pending_device_flow::Column::UserCode.eq(&normalized))
            .one(&txn)
            .await
            .context_to()?
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        // Check expiry
        if flow.expires_at <= now {
            bail!(DeviceFlowError::NotFound);
        }

        // Check already authorized
        if flow.status == DeviceAuthStatus::Authorized {
            bail!(DeviceFlowError::AlreadyAuthorized);
        }

        // Atomic update: only update if still pending and not expired (HA-safe)
        let result = PendingDeviceFlow::update_many()
            .col_expr(
                pending_device_flow::Column::Status,
                sea_orm::sea_query::Expr::value(DeviceAuthStatus::Authorized.as_str()),
            )
            .col_expr(
                pending_device_flow::Column::UserId,
                sea_orm::sea_query::Expr::value(user_id),
            )
            .filter(pending_device_flow::Column::Id.eq(flow.id))
            .filter(pending_device_flow::Column::Status.eq(DeviceAuthStatus::Pending.as_str()))
            .filter(pending_device_flow::Column::ExpiresAt.gt(now))
            .exec(&txn)
            .await
            .context_to()?;

        if result.rows_affected == 0 {
            // Another instance may have approved it, or it expired
            bail!(DeviceFlowError::AlreadyAuthorized);
        }

        txn.commit().await.context_to()?;
        Ok(())
    }

    /// Look up a pending device flow by user code.
    ///
    /// Returns the flow model if found, `None` if no matching flow exists.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceFlowError::Database`] on a DB error.
    pub async fn lookup_by_user_code(
        &self,
        user_code: &str,
    ) -> Result<Option<pending_device_flow::Model>> {
        let normalized = user_code.replace('-', "").to_uppercase();
        let flow = PendingDeviceFlow::find()
            .filter(pending_device_flow::Column::UserCode.eq(&normalized))
            .one(&self.db)
            .await
            .context_to()?;
        Ok(flow)
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
    #[expect(dead_code, reason = "available for future expiry-path tests")]
    async fn expire_flow(&self, device_code: &str) {
        use sea_orm::ActiveValue::Unchanged;
        let hash = hash_token(device_code);
        let flow = PendingDeviceFlow::find()
            .filter(pending_device_flow::Column::DeviceCodeHash.eq(&hash))
            .one(&self.db)
            .await
            .expect("expire_flow lookup")
            .expect("expire_flow flow not found");
        let expired_at =
            OffsetDateTime::now_utc() - time::Duration::seconds(DEVICE_CODE_TTL_SECONDS + 1);
        let model = pending_device_flow::ActiveModel {
            id: Unchanged(flow.id),
            expires_at: Set(expired_at),
            ..Default::default()
        };
        model.update(&self.db).await.expect("expire_flow update");
    }
}

/// Validate the OAuth `client_id` form parameter.
///
/// **Seam 3** — future migration replaces this function with a DB lookup
/// against an `oauth_clients` allowlist. The call sites in the routes layer
/// stay unchanged.
///
/// # Errors
///
/// Returns [`OAuthErrorCode::InvalidClient`] when `client_id` does not match
/// the hardcoded [`CLIENT_ID`] constant.
#[must_use = "discarding this Result silently skips client_id validation"]
pub fn validate_client_id(client_id: &str) -> std::result::Result<(), OAuthErrorCode> {
    if client_id == CLIENT_ID {
        Ok(())
    } else {
        Err(OAuthErrorCode::InvalidClient)
    }
}

/// Apply the requested `scope` parameter to a freshly-minted token.
///
/// **Seam 2** — today this is a no-op stub: scopes are recorded on the flow
/// row and echoed in audit, but no Permission narrowing happens. A future
/// migration replaces this body with a real scope→Permission map.
pub fn apply_scope_to_token(_token_id: Uuid, _scope: Option<&str>) {
    // intentional no-op
}

/// Mint a long-lived API access token for the given user inside the caller's
/// transaction.
///
/// **Seam 1** — future migration replaces this single function with
/// short-lived bearer + refresh-token issuance. Callers receive a
/// [`SecretString`] today; the future signature returns a `TokenPair`.
///
/// The function takes `txn: &impl ConnectionTrait` (not a pool) because the
/// row must land inside the same `BEGIN IMMEDIATE` transaction as the
/// `pending_device_flows` delete that authorises it — otherwise on SQLite the
/// pooled connection holding the txn would self-deadlock against a different
/// pooled connection issuing the insert, and on Postgres the insert would
/// fall outside the txn's atomicity envelope (orphan rows on crash between
/// the insert and the txn commit).
///
/// # Errors
///
/// Returns [`DeviceFlowError::TokenGeneration`] on RNG failure, or
/// [`DeviceFlowError::Database`] on a DB insert error.
#[must_use = "minted token must be returned to the caller"]
pub async fn issue_access_token<C: ConnectionTrait>(
    txn: &C,
    user_id: Uuid,
    token_name: &str,
) -> Result<(Uuid, SecretString)> {
    use sea_orm::Set;

    let raw = generate_secure_token()
        .map_err(|e| report!(DeviceFlowError::TokenGeneration(e.to_string())))?;
    let plaintext = format!("upk_{raw}");
    let token_hash = hash_token(&plaintext);
    let token_id = generate_uuid();
    let now = OffsetDateTime::now_utc();

    let model = api_token::ActiveModel {
        id: Set(token_id),
        user_id: Set(user_id),
        name: Set(token_name.to_string()),
        token_hash: Set(token_hash),
        created_at: Set(now),
        last_used_at: Set(None),
        revoked_at: Set(None),
    };
    model.insert(txn).await.context_to()?;

    Ok((token_id, SecretString::new(plaintext)))
}

/// Generate a user-friendly code: 8 uppercase consonants, formatted as XXXX-XXXX.
fn generate_user_code() -> String {
    let mut rng = rand::rng();
    let chars: [u8; 8] = std::array::from_fn(|_| {
        let idx = rng.random_range(0..USER_CODE_ALPHABET.len());
        USER_CODE_ALPHABET.get(idx).copied().unwrap_or(b'B')
    });

    let (first_half, second_half) = chars.split_at(4);
    let first: String = first_half.iter().map(|&b| b as char).collect();
    let second: String = second_half.iter().map(|&b| b as char).collect();

    format!("{first}-{second}")
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::string_slice,
        reason = "test assertions — ASCII-safe string slice in test assertions is idiomatic in tests"
    )]

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
        let (device_code, user_code) = store
            .create(Some("test-client".into()), None)
            .await
            .unwrap();

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

    /// A second test_db variant that also creates the `api_tokens` table so
    /// that `issue_access_token` (called inside `poll` on the Authorized path)
    /// can insert its row without a missing-table error.
    ///
    /// FK enforcement is disabled for this in-memory DB: we have no `users`
    /// parent rows and do not need them for the double-consume test. This is
    /// deliberately a unit test — no tenant, user, or session plumbing needed.
    async fn test_db_full() -> DatabaseConnection {
        use uptrakit_shared_db::entity::prelude::ApiToken;
        let db = test_db().await;
        // Disable FK enforcement so the missing `users` parent table does not
        // block api_token inserts during this unit test.
        db.execute_unprepared("PRAGMA foreign_keys = OFF")
            .await
            .expect("disable fk");
        let schema = Schema::new(db.get_database_backend());
        let stmt = schema.create_table_from_entity(ApiToken);
        db.execute(&stmt).await.expect("create api_tokens table");
        db
    }

    #[tokio::test]
    async fn slow_down_when_polled_too_fast() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, _) = store.create(None, None).await.unwrap();
        let t0 = OffsetDateTime::now_utc();

        let first = store.poll(&device_code, t0).await.unwrap();
        assert!(matches!(first, PollOutcome::Pending));

        let t1 = t0 + time::Duration::seconds(2); // less than POLL_INTERVAL_SECONDS (5)
        let second = store.poll(&device_code, t1).await.unwrap();
        assert!(matches!(second, PollOutcome::SlowDown { .. }));
    }

    #[tokio::test]
    async fn slow_down_returns_bumped_interval() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, _) = store.create(None, None).await.unwrap();
        let t0 = OffsetDateTime::now_utc();
        let _ = store.poll(&device_code, t0).await.unwrap();

        let t1 = t0 + time::Duration::seconds(1);
        let outcome = store.poll(&device_code, t1).await.unwrap();
        assert!(
            matches!(outcome, PollOutcome::SlowDown { bumped_interval } if bumped_interval == 10),
            "expected bumped_interval = 10, got {outcome:?}"
        );

        // Another fast poll — should bump again to 15.
        let t2 = t1 + time::Duration::seconds(1);
        let outcome = store.poll(&device_code, t2).await.unwrap();
        assert!(
            matches!(outcome, PollOutcome::SlowDown { bumped_interval } if bumped_interval == 15),
            "expected bumped_interval = 15, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn last_polled_at_updates_on_each_poll() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db.clone());
        let (device_code, _) = store.create(None, None).await.unwrap();
        let t0 = OffsetDateTime::now_utc();
        store.poll(&device_code, t0).await.unwrap();
        let t1 = t0 + time::Duration::seconds(60);
        store.poll(&device_code, t1).await.unwrap();

        // Inspect via a direct entity read.
        let hash = crate::auth::token::hash_token(&device_code);
        let flow = uptrakit_shared_db::entity::pending_device_flow::Entity::find()
            .filter(
                uptrakit_shared_db::entity::pending_device_flow::Column::DeviceCodeHash.eq(&hash),
            )
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        // poll stores `last_polled_at = now` where `now` is the injected parameter.
        assert_eq!(flow.last_polled_at, Some(t1));
    }

    #[tokio::test]
    async fn unknown_device_code_returns_expired_token() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let outcome = store
            .poll("not-a-known-code", OffsetDateTime::now_utc())
            .await
            .unwrap();
        assert!(matches!(outcome, PollOutcome::Unknown));
    }

    #[tokio::test]
    async fn malformed_device_code_returns_invalid_grant() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let outcome = store.poll("", OffsetDateTime::now_utc()).await.unwrap();
        assert!(matches!(outcome, PollOutcome::MalformedDeviceCode));
    }

    #[tokio::test]
    async fn deny_marks_flow_denied_and_sets_denied_by() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db.clone());
        let (_device_code, user_code) = store.create(None, None).await.unwrap();
        let denier = Uuid::now_v7();

        store.deny(&user_code, denier).await.unwrap();

        let normalized = user_code.replace('-', "").to_uppercase();
        let flow = uptrakit_shared_db::entity::pending_device_flow::Entity::find()
            .filter(
                uptrakit_shared_db::entity::pending_device_flow::Column::UserCode.eq(&normalized),
            )
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(flow.status, DeviceAuthStatus::Denied);
        assert_eq!(flow.denied_by, Some(denier));
        assert_eq!(flow.user_id, None);
    }

    #[tokio::test]
    async fn poll_after_deny_returns_access_denied() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, user_code) = store.create(None, None).await.unwrap();
        let denier = Uuid::now_v7();
        store.deny(&user_code, denier).await.unwrap();

        let outcome = store
            .poll(&device_code, OffsetDateTime::now_utc())
            .await
            .unwrap();
        assert!(matches!(outcome, PollOutcome::Denied));
    }

    #[tokio::test]
    async fn concurrent_poll_does_not_double_consume() {
        let db = test_db_full().await;
        let store = DeviceFlowStore::new(db);
        let (device_code, user_code) = store.create(None, None).await.unwrap();
        let user_id = Uuid::now_v7();
        store.approve(&user_code, user_id).await.unwrap();
        let now = OffsetDateTime::now_utc();

        let store2 = store.clone();
        let dc2 = device_code.clone();
        let (a, b) = tokio::join!(store.poll(&device_code, now), store2.poll(&dc2, now),);

        let outcomes = [a.unwrap(), b.unwrap()];
        let authorized_count = outcomes
            .iter()
            .filter(|o| matches!(o, PollOutcome::Authorized { .. }))
            .count();
        let unknown_count = outcomes
            .iter()
            .filter(|o| matches!(o, PollOutcome::Unknown))
            .count();
        assert_eq!(authorized_count, 1, "exactly one authorized: {outcomes:?}");
        assert_eq!(unknown_count, 1, "exactly one unknown: {outcomes:?}");
    }

    #[tokio::test]
    async fn concurrent_approve_and_deny_resolves_atomically() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (_device_code, user_code) = store.create(None, None).await.unwrap();
        let approver = Uuid::now_v7();
        let denier = Uuid::now_v7();

        let s1 = store.clone();
        let s2 = store.clone();
        let uc1 = user_code.clone();
        let uc2 = user_code.clone();
        let (a, b) = tokio::join!(s1.approve(&uc1, approver), s2.deny(&uc2, denier),);

        let winners = [a.is_ok(), b.is_ok()];
        assert_eq!(
            winners.iter().filter(|x| **x).count(),
            1,
            "exactly one wins"
        );
    }

    #[tokio::test]
    async fn test_approve_normalizes_code() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (_device_code, user_code) = store.create(None, None).await.unwrap();
        let user_id = Uuid::now_v7();

        // Approve with lowercase and hyphen
        let lower = user_code.to_lowercase();
        store.approve(&lower, user_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_approve_already_authorized() {
        let db = test_db().await;
        let store = DeviceFlowStore::new(db);
        let (_device_code, user_code) = store.create(None, None).await.unwrap();
        let user_id = Uuid::now_v7();

        store.approve(&user_code, user_id).await.unwrap();

        let err = store.approve(&user_code, user_id).await.unwrap_err();
        assert!(matches!(
            err.current_context(),
            DeviceFlowError::AlreadyAuthorized
        ));
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
