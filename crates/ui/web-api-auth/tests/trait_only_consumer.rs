//! Prove that a downstream consumer can use [`SessionOps`] and [`ApiTokenOps`]
//! with only `uptrakit-web-api-auth` as a dependency — no `axum`, no `web-api`,
//! no `DbState`, no sub-state types.
//!
//! Compilation is the primary assertion. If this file compiles and the test
//! runs, the trait-only dependency boundary holds.

use async_trait::async_trait;
use std::sync::Arc;
use uptrakit_web_api_auth::auth::AuthMethod;
use uptrakit_web_api_auth::auth::api_token::{ApiTokenInfo, CreatedApiToken};
use uptrakit_web_api_auth::auth::session::VerifiedSession;
use uptrakit_web_api_auth::auth::{ApiTokenOps, SessionOps};

struct Consumer {
    session: Arc<dyn SessionOps>,
    tokens: Arc<dyn ApiTokenOps>,
}

impl Consumer {
    fn new(session: Arc<dyn SessionOps>, tokens: Arc<dyn ApiTokenOps>) -> Self {
        Self { session, tokens }
    }
}

struct StubSessionOps;

#[async_trait]
impl SessionOps for StubSessionOps {
    async fn create_refresh_token(
        &self,
        _user_id: uuid::Uuid,
        _auth_method: AuthMethod,
        _user_agent: Option<String>,
        _ip_address: Option<String>,
    ) -> uptrakit_web_api_auth::auth::Result<String> {
        Ok("stub-token".to_string())
    }

    async fn verify_refresh_token(
        &self,
        _token: &str,
    ) -> uptrakit_web_api_auth::auth::Result<VerifiedSession> {
        unimplemented!("stub")
    }

    async fn rotate_refresh_token(
        &self,
        _token: &str,
    ) -> uptrakit_web_api_auth::auth::Result<(VerifiedSession, String)> {
        unimplemented!("stub")
    }

    async fn revoke_refresh_token(&self, _token: &str) -> uptrakit_web_api_auth::auth::Result<()> {
        Ok(())
    }

    async fn delete_user_sessions(
        &self,
        _user_id: uuid::Uuid,
    ) -> uptrakit_web_api_auth::auth::Result<()> {
        Ok(())
    }

    async fn delete_user_sessions_except(
        &self,
        _user_id: uuid::Uuid,
        _except_session_id: uuid::Uuid,
    ) -> uptrakit_web_api_auth::auth::Result<()> {
        Ok(())
    }

    async fn cleanup_expired_sessions(&self) -> uptrakit_web_api_auth::auth::Result<u64> {
        Ok(0)
    }
}

struct StubApiTokenOps;

#[async_trait]
impl ApiTokenOps for StubApiTokenOps {
    async fn create_token(
        &self,
        _user_id: uuid::Uuid,
        _name: &str,
    ) -> uptrakit_web_api_auth::auth::Result<CreatedApiToken> {
        unimplemented!("stub")
    }

    async fn list_tokens(
        &self,
        _user_id: uuid::Uuid,
    ) -> uptrakit_web_api_auth::auth::Result<Vec<ApiTokenInfo>> {
        Ok(vec![])
    }

    async fn revoke_token(
        &self,
        _token_id: uuid::Uuid,
        _user_id: uuid::Uuid,
    ) -> uptrakit_web_api_auth::auth::Result<()> {
        Ok(())
    }

    async fn verify_token(
        &self,
        _plaintext: &str,
    ) -> uptrakit_web_api_auth::auth::Result<(uuid::Uuid, uuid::Uuid, String)> {
        Ok((
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            "stub-token".to_string(),
        ))
    }
}

#[tokio::test]
async fn trait_only_consumer_compiles() {
    let consumer = Consumer::new(Arc::new(StubSessionOps), Arc::new(StubApiTokenOps));
    let token = consumer
        .session
        .create_refresh_token(uuid::Uuid::new_v4(), AuthMethod::Password, None, None)
        .await
        .expect("stub should succeed");
    assert_eq!(token, "stub-token");

    let result = consumer.tokens.verify_token("any").await;
    assert!(result.is_ok());
}
