use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{api_rate_limit, pending_device_flow, scheduled_task};

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

        txn.commit().await.context_to()?;

        tracing::debug!("auth state cleanup completed");
        Ok(())
    }
}
