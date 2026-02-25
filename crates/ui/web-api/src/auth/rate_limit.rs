use rootcause::prelude::*;
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{api_rate_limit, prelude::ApiRateLimit};
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
pub enum RateLimitError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, Report<RateLimitError>>;

impl_report_conversion!(sea_orm::DbErr => RateLimitError::Database);

/// Outcome of a rate limit check.
#[derive(Debug, Clone, PartialEq)]
pub enum RateLimitOutcome {
    /// Request is allowed.
    Allowed,
    /// Request is rate-limited. The caller should retry after `retry_after_secs`.
    Limited { retry_after_secs: u64 },
}

/// Database-backed rate limiter for API endpoints.
///
/// Uses a sliding-window counter per key (typically `{path}:{ip}`).
/// HA-safe: all state is in the database, so multiple controller instances
/// share the same rate limit buckets.
#[derive(Clone)]
pub struct RateLimitStore {
    db: DatabaseConnection,
}

impl RateLimitStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Check (and count) a request against the rate limit for `key`.
    ///
    /// - `max_requests`: maximum number of requests allowed in the window.
    /// - `window_secs`: window duration in seconds.
    ///
    /// Uses a single atomic SQL upsert to avoid TOCTOU races: the counter is
    /// incremented (or reset) in one statement, then read back to decide.
    ///
    /// Returns `Allowed` if the request is within the limit, or `Limited`
    /// with a `retry_after_secs` value if the limit has been exceeded.
    pub async fn check_rate_limit(
        &self,
        key: &str,
        max_requests: i32,
        window_secs: i64,
    ) -> Result<RateLimitOutcome> {
        let now = OffsetDateTime::now_utc();
        let window = time::Duration::seconds(window_secs);
        let threshold = now - window;
        let expires_at = now + time::Duration::seconds(window_secs * 2);

        // Atomic upsert: INSERT or UPDATE in a single statement.
        // If the row doesn't exist: insert with count=1.
        // If the row exists with a valid window (window_start >= threshold): increment.
        // If the row exists with an expired window: reset to count=1 with new timestamps.
        //
        // Raw SQL is required because SeaORM's on_conflict builder doesn't
        // support CASE WHEN expressions. The statement is fully parameterized.
        let backend = self.db.get_database_backend();
        let sql = match backend {
            sea_orm::DatabaseBackend::MySql => {
                r#"INSERT INTO api_rate_limits (`key`, request_count, window_start, expires_at)
                   VALUES (?, 1, ?, ?)
                   ON DUPLICATE KEY UPDATE
                     request_count = CASE
                       WHEN window_start >= ? THEN request_count + 1
                       ELSE 1
                     END,
                     window_start = CASE
                       WHEN window_start >= ? THEN window_start
                       ELSE VALUES(window_start)
                     END,
                     expires_at = VALUES(expires_at)"#
            }
            // SQLite and PostgreSQL both use $N positional params and ON CONFLICT
            _ => {
                r#"INSERT INTO api_rate_limits ("key", request_count, window_start, expires_at)
                   VALUES ($1, 1, $2, $3)
                   ON CONFLICT ("key") DO UPDATE SET
                     request_count = CASE
                       WHEN api_rate_limits.window_start >= $4 THEN api_rate_limits.request_count + 1
                       ELSE 1
                     END,
                     window_start = CASE
                       WHEN api_rate_limits.window_start >= $4 THEN api_rate_limits.window_start
                       ELSE excluded.window_start
                     END,
                     expires_at = excluded.expires_at"#
            }
        };

        let params = match backend {
            sea_orm::DatabaseBackend::MySql => {
                // MySQL uses ? placeholders; threshold is repeated for both CASE WHEN clauses.
                vec![
                    key.into(),
                    now.into(),
                    expires_at.into(),
                    threshold.into(),
                    threshold.into(),
                ]
            }
            _ => {
                vec![key.into(), now.into(), expires_at.into(), threshold.into()]
            }
        };

        let stmt = sea_orm::Statement::from_sql_and_values(backend, sql, params);
        self.db.execute_raw(stmt).await.context_to()?;

        // Read back the current count to decide the outcome.
        let row = ApiRateLimit::find_by_id(key)
            .one(&self.db)
            .await
            .context_to()?;

        match row {
            Some(row) if row.request_count > max_requests => {
                let window_end = row.window_start + window;
                let remaining = window_end - now;
                let retry_after = Ord::max(remaining.whole_seconds(), 1) as u64;
                Ok(RateLimitOutcome::Limited {
                    retry_after_secs: retry_after,
                })
            }
            _ => Ok(RateLimitOutcome::Allowed),
        }
    }

    /// Remove expired rate limit entries. Fire-and-forget.
    pub async fn cleanup_expired(&self) {
        let now = OffsetDateTime::now_utc();
        let result = ApiRateLimit::delete_many()
            .filter(api_rate_limit::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await;

        if let Err(e) = result {
            tracing::warn!("failed to clean up expired rate limit entries: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, Set};

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    #[tokio::test]
    async fn test_first_request_allowed() {
        let db = test_db().await;
        let store = RateLimitStore::new(db);

        let outcome = store
            .check_rate_limit("test:127.0.0.1", 10, 60)
            .await
            .expect("check");
        assert_eq!(outcome, RateLimitOutcome::Allowed);
    }

    #[tokio::test]
    async fn test_requests_within_limit_allowed() {
        let db = test_db().await;
        let store = RateLimitStore::new(db);

        for _ in 0..10 {
            let outcome = store
                .check_rate_limit("test:127.0.0.1", 10, 60)
                .await
                .expect("check");
            assert_eq!(outcome, RateLimitOutcome::Allowed);
        }
    }

    #[tokio::test]
    async fn test_exceeds_limit_returns_limited() {
        let db = test_db().await;
        let store = RateLimitStore::new(db);

        // Use up all 10 requests
        for _ in 0..10 {
            let outcome = store
                .check_rate_limit("test:127.0.0.1", 10, 60)
                .await
                .expect("check");
            assert_eq!(outcome, RateLimitOutcome::Allowed);
        }

        // 11th should be limited
        let outcome = store
            .check_rate_limit("test:127.0.0.1", 10, 60)
            .await
            .expect("check");
        assert!(
            matches!(outcome, RateLimitOutcome::Limited { .. }),
            "expected Limited, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn test_retry_after_is_positive() {
        let db = test_db().await;
        let store = RateLimitStore::new(db);

        for _ in 0..10 {
            store
                .check_rate_limit("test:127.0.0.1", 10, 60)
                .await
                .expect("check");
        }

        match store
            .check_rate_limit("test:127.0.0.1", 10, 60)
            .await
            .expect("check")
        {
            RateLimitOutcome::Limited { retry_after_secs } => {
                assert!(
                    retry_after_secs > 0,
                    "retry_after should be positive, got {retry_after_secs}"
                );
                assert!(
                    retry_after_secs <= 60,
                    "retry_after should be <= window, got {retry_after_secs}"
                );
            }
            other => panic!("expected Limited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_expired_window_resets() {
        let db = test_db().await;
        let store = RateLimitStore::new(db.clone());

        // Use up all 10 requests
        for _ in 0..10 {
            store
                .check_rate_limit("test:127.0.0.1", 10, 60)
                .await
                .expect("check");
        }

        // Verify we're limited
        let outcome = store
            .check_rate_limit("test:127.0.0.1", 10, 60)
            .await
            .expect("check");
        assert!(matches!(outcome, RateLimitOutcome::Limited { .. }));

        // Backdate the window_start to make the window expired
        let past = OffsetDateTime::now_utc() - time::Duration::seconds(120);
        let model = api_rate_limit::ActiveModel {
            key: Set("test:127.0.0.1".to_string()),
            window_start: Set(past),
            request_count: Set(10),
            expires_at: Set(past + time::Duration::seconds(120)),
        };
        model.update(&db).await.expect("backdate");

        // Should be allowed again (window expired, resets)
        let outcome = store
            .check_rate_limit("test:127.0.0.1", 10, 60)
            .await
            .expect("check");
        assert_eq!(outcome, RateLimitOutcome::Allowed);
    }

    #[tokio::test]
    async fn test_different_keys_independent() {
        let db = test_db().await;
        let store = RateLimitStore::new(db);

        // Exhaust limit for key A
        for _ in 0..3 {
            store
                .check_rate_limit("a:127.0.0.1", 3, 60)
                .await
                .expect("check");
        }

        // Key A is limited
        let outcome = store
            .check_rate_limit("a:127.0.0.1", 3, 60)
            .await
            .expect("check");
        assert!(matches!(outcome, RateLimitOutcome::Limited { .. }));

        // Key B is still allowed
        let outcome = store
            .check_rate_limit("b:127.0.0.1", 3, 60)
            .await
            .expect("check");
        assert_eq!(outcome, RateLimitOutcome::Allowed);
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let db = test_db().await;
        let store = RateLimitStore::new(db.clone());

        // Insert with a past expires_at
        let past = OffsetDateTime::now_utc() - time::Duration::seconds(10);
        let model = api_rate_limit::ActiveModel {
            key: Set("expired:1.2.3.4".to_string()),
            request_count: Set(5),
            window_start: Set(past - time::Duration::seconds(60)),
            expires_at: Set(past),
        };
        model.insert(&db).await.expect("insert");

        // Insert one that's not expired
        let future = OffsetDateTime::now_utc() + time::Duration::seconds(120);
        let model = api_rate_limit::ActiveModel {
            key: Set("active:1.2.3.4".to_string()),
            request_count: Set(1),
            window_start: Set(OffsetDateTime::now_utc()),
            expires_at: Set(future),
        };
        model.insert(&db).await.expect("insert");

        store.cleanup_expired().await;

        // Expired row should be gone
        let expired = ApiRateLimit::find_by_id("expired:1.2.3.4")
            .one(&db)
            .await
            .expect("find");
        assert!(expired.is_none(), "expired entry should be cleaned up");

        // Active row should remain
        let active = ApiRateLimit::find_by_id("active:1.2.3.4")
            .one(&db)
            .await
            .expect("find");
        assert!(active.is_some(), "active entry should remain");
    }
}
