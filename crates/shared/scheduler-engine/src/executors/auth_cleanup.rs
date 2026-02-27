use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{api_rate_limit, pending_device_flow, scheduled_task};

use crate::executor::TaskExecutor;

/// Cleans expired auth state from DB-backed stores.
///
/// Uses direct DB queries instead of store wrappers so the scheduler engine
/// does not depend on `uptrakit-web-api`. Does NOT clean `TokenDenylist`
/// (that stays per-controller, in-memory).
pub struct AuthCleanupExecutor {
    db: DatabaseConnection,
}

impl AuthCleanupExecutor {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Delete expired OIDC flow entries.
    #[cfg(feature = "oidc")]
    async fn cleanup_oidc_flows(&self) {
        use uptrakit_shared_db::entity::{
            pending_account_link, pending_oidc_flow, pending_oidc_registration,
            pending_oidc_token_exchange,
        };

        let now = OffsetDateTime::now_utc();

        if let Err(e) = PendingOidcFlow::delete_many()
            .filter(pending_oidc_flow::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await
        {
            tracing::warn!(error = %e, "failed to clean up expired OIDC flows");
        }

        if let Err(e) = PendingAccountLink::delete_many()
            .filter(pending_account_link::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await
        {
            tracing::warn!(error = %e, "failed to clean up expired account links");
        }

        if let Err(e) = PendingOidcTokenExchange::delete_many()
            .filter(pending_oidc_token_exchange::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await
        {
            tracing::warn!(error = %e, "failed to clean up expired OIDC token exchanges");
        }

        if let Err(e) = PendingOidcRegistration::delete_many()
            .filter(pending_oidc_registration::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await
        {
            tracing::warn!(error = %e, "failed to clean up expired OIDC registrations");
        }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for AuthCleanupExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> crate::error::Result<()> {
        let now = OffsetDateTime::now_utc();

        #[cfg(feature = "oidc")]
        self.cleanup_oidc_flows().await;

        if let Err(e) = PendingDeviceFlow::delete_many()
            .filter(pending_device_flow::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await
        {
            tracing::warn!(error = %e, "failed to clean up expired device flows");
        }

        if let Err(e) = ApiRateLimit::delete_many()
            .filter(api_rate_limit::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await
        {
            tracing::warn!(error = %e, "failed to clean up expired rate limit entries");
        }

        tracing::debug!("auth state cleanup completed");
        Ok(())
    }
}
