use uptrakit_shared_db::entity::scheduled_task;
use uptrakit_web_api::auth::device_flow::DeviceFlowStore;
use uptrakit_web_api::auth::oidc_state::{
    AccountLinkStore, OidcFlowStore, OidcRegistrationStore, OidcTokenExchangeStore,
};
use uptrakit_web_api::auth::rate_limit::RateLimitStore;

use crate::scheduler::executor::TaskExecutor;

/// Cleans expired auth state from DB-backed stores.
///
/// Does NOT clean `TokenDenylist` (that stays per-controller, in-memory).
pub struct AuthCleanupExecutor {
    oidc_flow_store: OidcFlowStore,
    account_link_store: AccountLinkStore,
    oidc_token_exchange_store: OidcTokenExchangeStore,
    oidc_registration_store: OidcRegistrationStore,
    device_flow_store: DeviceFlowStore,
    rate_limit_store: RateLimitStore,
}

impl AuthCleanupExecutor {
    pub fn new(
        oidc_flow_store: OidcFlowStore,
        account_link_store: AccountLinkStore,
        oidc_token_exchange_store: OidcTokenExchangeStore,
        oidc_registration_store: OidcRegistrationStore,
        device_flow_store: DeviceFlowStore,
        rate_limit_store: RateLimitStore,
    ) -> Self {
        Self {
            oidc_flow_store,
            account_link_store,
            oidc_token_exchange_store,
            oidc_registration_store,
            device_flow_store,
            rate_limit_store,
        }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for AuthCleanupExecutor {
    async fn execute(&self, _task: &scheduled_task::Model) -> Result<(), String> {
        self.oidc_flow_store.cleanup_expired().await;
        self.account_link_store.cleanup_expired().await;
        self.oidc_token_exchange_store.cleanup_expired().await;
        self.oidc_registration_store.cleanup_expired().await;
        self.device_flow_store.cleanup_expired().await;
        self.rate_limit_store.cleanup_expired().await;
        tracing::debug!("auth state cleanup completed");
        Ok(())
    }
}
