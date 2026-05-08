use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{
    api_rate_limit, email_change_request, pending_device_flow, scheduled_task,
};

use crate::executor::TaskExecutor;

/// Cleans expired auth state from DB-backed stores.
///
/// Uses direct DB queries instead of store wrappers so the scheduler engine
/// does not depend on `uptrakit-web-api`. Does NOT clean `TokenDenylist`
/// (that stays per-controller, in-memory).
///
/// All DELETE statements run inside a single database transaction so that the
/// cleanup is atomic: either all expired records are removed in one round trip,
/// or none are (the transaction rolls back on error). This reduces the number
/// of PostgreSQL round trips and prevents partial cleanup states.
pub struct AuthCleanupExecutor {
    db: DatabaseConnection,
}

impl AuthCleanupExecutor {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for AuthCleanupExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
        let now = OffsetDateTime::now_utc();

        let txn = self.db.begin().await.context_to()?;

        #[cfg(feature = "oidc")]
        {
            use uptrakit_shared_db::entity::{
                pending_account_link, pending_oidc_flow, pending_oidc_registration,
                pending_oidc_token_exchange,
            };

            PendingOidcFlow::delete_many()
                .filter(pending_oidc_flow::Column::ExpiresAt.lt(now))
                .exec(&txn)
                .await
                .context_to()?;

            PendingAccountLink::delete_many()
                .filter(pending_account_link::Column::ExpiresAt.lt(now))
                .exec(&txn)
                .await
                .context_to()?;

            PendingOidcTokenExchange::delete_many()
                .filter(pending_oidc_token_exchange::Column::ExpiresAt.lt(now))
                .exec(&txn)
                .await
                .context_to()?;

            PendingOidcRegistration::delete_many()
                .filter(pending_oidc_registration::Column::ExpiresAt.lt(now))
                .exec(&txn)
                .await
                .context_to()?;
        }

        PendingDeviceFlow::delete_many()
            .filter(pending_device_flow::Column::ExpiresAt.lt(now))
            .exec(&txn)
            .await
            .context_to()?;

        ApiRateLimit::delete_many()
            .filter(api_rate_limit::Column::ExpiresAt.lt(now))
            .exec(&txn)
            .await
            .context_to()?;

        EmailChangeRequest::delete_many()
            .filter(email_change_request::Column::ExpiresAt.lt(now))
            .exec(&txn)
            .await
            .context_to()?;

        txn.commit().await.context_to()?;

        tracing::debug!("auth state cleanup completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, Set};
    use uptrakit_shared_db::migration::run_migrations;

    async fn setup_db() -> DatabaseConnection {
        uptrakit_crypto::enable_plaintext_mode();
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        run_migrations(&db).await.expect("run migrations");
        db
    }

    fn make_task(_db: &DatabaseConnection) -> scheduled_task::Model {
        let tenant_id = uuid::Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        scheduled_task::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id,
            task_type: uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType::AuthCleanup,
            interval_seconds: 3600,
            jitter_seconds: 0,
            enabled: true,
            task_config: None,
            last_run_at: None,
            next_run_at: now,
            locked_by: None,
            locked_at: None,
            last_error: None,
            run_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn empty_db_returns_ok() {
        let db = setup_db().await;
        let executor = AuthCleanupExecutor::new(db.clone());
        let task = make_task(&db);
        executor
            .execute(&task)
            .await
            .expect("execute should succeed");
    }

    #[tokio::test]
    async fn deletes_expired_device_flow_and_rate_limit() {
        let db = setup_db().await;
        let now = OffsetDateTime::now_utc();
        let past = now - time::Duration::hours(1);
        let future = now + time::Duration::hours(1);

        // Insert an expired pending_device_flow.
        pending_device_flow::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            device_code_hash: Set("expired-hash".to_string()),
            user_code: Set("EXP1".to_string()),
            status: Set(uptrakit_shared_types::DeviceAuthStatus::Pending),
            user_id: Set(None),
            client_name: Set(None),
            created_at: Set(past),
            expires_at: Set(past),
        }
        .insert(&db)
        .await
        .expect("insert expired flow");

        // Insert a fresh pending_device_flow.
        pending_device_flow::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            device_code_hash: Set("fresh-hash".to_string()),
            user_code: Set("FRS1".to_string()),
            status: Set(uptrakit_shared_types::DeviceAuthStatus::Pending),
            user_id: Set(None),
            client_name: Set(None),
            created_at: Set(now),
            expires_at: Set(future),
        }
        .insert(&db)
        .await
        .expect("insert fresh flow");

        // Insert an expired api_rate_limit.
        api_rate_limit::ActiveModel {
            key: Set("expired-key".to_string()),
            request_count: Set(10),
            window_start: Set(past),
            expires_at: Set(past),
        }
        .insert(&db)
        .await
        .expect("insert expired rate limit");

        // Insert a fresh api_rate_limit.
        api_rate_limit::ActiveModel {
            key: Set("fresh-key".to_string()),
            request_count: Set(1),
            window_start: Set(now),
            expires_at: Set(future),
        }
        .insert(&db)
        .await
        .expect("insert fresh rate limit");

        let executor = AuthCleanupExecutor::new(db.clone());
        let task = make_task(&db);
        executor
            .execute(&task)
            .await
            .expect("execute should succeed");

        // Verify: only fresh rows remain.
        let flows = PendingDeviceFlow::find()
            .all(&db)
            .await
            .expect("query flows");
        assert_eq!(flows.len(), 1, "only the fresh device flow should remain");
        assert_eq!(flows[0].device_code_hash, "fresh-hash");

        let limits = ApiRateLimit::find()
            .all(&db)
            .await
            .expect("query rate limits");
        assert_eq!(limits.len(), 1, "only the fresh rate limit should remain");
        assert_eq!(limits[0].key, "fresh-key");
    }

    #[tokio::test]
    async fn fresh_rows_are_not_deleted() {
        let db = setup_db().await;
        let now = OffsetDateTime::now_utc();
        let future = now + time::Duration::hours(2);

        pending_device_flow::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            device_code_hash: Set("kept-hash".to_string()),
            user_code: Set("KPT1".to_string()),
            status: Set(uptrakit_shared_types::DeviceAuthStatus::Pending),
            user_id: Set(None),
            client_name: Set(None),
            created_at: Set(now),
            expires_at: Set(future),
        }
        .insert(&db)
        .await
        .expect("insert fresh flow");

        let executor = AuthCleanupExecutor::new(db.clone());
        let task = make_task(&db);
        executor
            .execute(&task)
            .await
            .expect("execute should succeed");

        let flows = PendingDeviceFlow::find()
            .all(&db)
            .await
            .expect("query flows");
        assert_eq!(flows.len(), 1, "fresh device flow must not be deleted");
    }

    #[tokio::test]
    async fn deletes_expired_email_change_requests() {
        use uptrakit_shared_db::entity::{email_change_request, user};
        use uptrakit_shared_types::MaskedEmail;

        let db = setup_db().await;
        let now = OffsetDateTime::now_utc();
        let past = now - time::Duration::hours(25);
        let future = now + time::Duration::hours(24);

        // Insert test users (required by FK).
        let user_id = uuid::Uuid::now_v7();
        let user2_id = uuid::Uuid::now_v7();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(MaskedEmail::new("test-ecr1@example.com")),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user 1");

        user::ActiveModel {
            id: Set(user2_id),
            email: Set(MaskedEmail::new("test-ecr2@example.com")),
            first_name: Set("Test2".to_string()),
            last_name: Set("User2".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user 2");

        // Insert expired request.
        email_change_request::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            user_id: Set(user_id),
            new_email: Set(uptrakit_crypto::EncryptedString::plaintext_for_test(
                "expired@example.com".to_string(),
            )),
            token_hash: Set("expired-hash".to_string()),
            expires_at: Set(past),
            created_at: Set(past),
        }
        .insert(&db)
        .await
        .expect("insert expired request");

        // Insert fresh request.
        email_change_request::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            user_id: Set(user2_id),
            new_email: Set(uptrakit_crypto::EncryptedString::plaintext_for_test(
                "fresh@example.com".to_string(),
            )),
            token_hash: Set("fresh-hash".to_string()),
            expires_at: Set(future),
            created_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert fresh request");

        let executor = AuthCleanupExecutor::new(db.clone());
        let task = make_task(&db);
        executor
            .execute(&task)
            .await
            .expect("execute should succeed");

        let remaining = EmailChangeRequest::find().all(&db).await.expect("query");
        assert_eq!(remaining.len(), 1, "only fresh request should remain");
        assert_eq!(remaining[0].token_hash, "fresh-hash");
    }
}
