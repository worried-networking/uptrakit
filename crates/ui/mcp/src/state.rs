use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use uptrakit_audit_log::AuditEmitter;
use uptrakit_controller_core::auth::AuthState;
use uptrakit_controller_core::db::DbState;
use uptrakit_controller_core::settings::Settings;
use uptrakit_controller_core::update::UpdateDispatcher;
use uptrakit_web_api_types::oauth::{CanonicalUrlConfig, McpOAuthJwtVerifier};

/// Focused state struct for the MCP server.
///
/// Contains only the controller-core fields needed by MCP tool handlers and
/// auth middleware. Has no dependency on `uptrakit-web-api` or `axum`.
///
/// At controller startup one `ControllerUpdateDispatcher` `Arc` is cloned into
/// both `AppState` and `McpState`.
///
/// `#[non_exhaustive]`: prevents external struct literal construction and forces
/// exhaustive pattern match sites to add `..`. Callers use `McpState::new(…)`.
///
/// `access_engine` is the controller's engine instance — never construct a
/// second one (one cache, one invalidation listener).
#[non_exhaustive]
#[derive(Clone)]
pub struct McpState {
    pub db: DbState,
    pub auth: AuthState,
    pub access_engine: Arc<uptrakit_controller_core::access::AccessEngine>,
    pub settings: Settings,
    pub default_tenant_id: Uuid,
    pub controller_id: Uuid,
    pub audit_emitter: AuditEmitter,
    pub shutdown_token: CancellationToken,
    pub update_dispatcher: Arc<dyn UpdateDispatcher>,
    pub oauth_enabled: bool,
    pub oauth_verifier: Option<Arc<McpOAuthJwtVerifier>>,
    pub oauth_canonical: Option<Arc<CanonicalUrlConfig>>,
}

impl McpState {
    /// Creates a new [`McpState`].
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "McpState::new is the sole construction path for this #[non_exhaustive] struct; a builder would add indirection for a single call site"
    )]
    pub fn new(
        db: DbState,
        auth: AuthState,
        access_engine: Arc<uptrakit_controller_core::access::AccessEngine>,
        settings: Settings,
        default_tenant_id: Uuid,
        controller_id: Uuid,
        audit_emitter: AuditEmitter,
        shutdown_token: CancellationToken,
        update_dispatcher: Arc<dyn UpdateDispatcher>,
        oauth_enabled: bool,
        oauth_verifier: Option<Arc<McpOAuthJwtVerifier>>,
        oauth_canonical: Option<Arc<CanonicalUrlConfig>>,
    ) -> Self {
        Self {
            db,
            auth,
            access_engine,
            settings,
            default_tenant_id,
            controller_id,
            audit_emitter,
            shutdown_token,
            update_dispatcher,
            oauth_enabled,
            oauth_verifier,
            oauth_canonical,
        }
    }
}
