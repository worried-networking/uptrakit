//! MCP OAuth 2.1 authorization-server business logic.
//!
//! This module contains the state struct, sub-service implementations, and
//! helpers for the MCP OAuth authorization server.  The HTTP route handlers
//! live in `crates/ui/web-api/src/routes/oauth/` and delegate to the types
//! defined here.

pub mod boot;
pub mod canonical_url;
pub mod cimd;
pub mod cimd_parser;
pub mod http_responses;
pub mod jwt;
pub mod pkce;
pub mod rate_limit;
pub mod services;

use std::sync::Arc;

use time::OffsetDateTime;

use crate::oauth::canonical_url::{CanonicalUrlConfig, disabled_placeholder};
use crate::oauth::jwt::{McpOAuthJwtSigner, McpOAuthJwtVerifier};

/// Runtime state for the MCP OAuth 2.1 authorization server.
///
/// Stored as a field on [`crate::app_state::AppState`].  When
/// `enabled = false` all OAuth routes return `404 Not Found` and the
/// remaining fields carry inert placeholder values (see [`OAuthState::disabled`]).
///
/// `#[non_exhaustive]`: new fields may be added as the implementation grows
/// (e.g. consent store, client registry).  External crates must use
/// [`OAuthState::disabled`] rather than constructing the struct directly.
#[non_exhaustive]
#[derive(Clone)]
pub struct OAuthState {
    /// Whether the MCP OAuth authorization server is active.
    ///
    /// When `false`, all `/oauth/*` routes must return `404 Not Found`.
    pub enabled: bool,
    /// Canonical-URL configuration: issuer, primary resource, accepted audience set.
    pub canonical: CanonicalUrlConfig,
    /// JWT signer used to issue access tokens and authorization codes.
    pub signer: Arc<McpOAuthJwtSigner>,
    /// JWT verifier used to validate access tokens on protected resource requests.
    pub verifier: Arc<McpOAuthJwtVerifier>,
    /// Pluggable clock for deterministic testing.
    ///
    /// Production code passes `Arc::new(OffsetDateTime::now_utc)`.
    /// Test code passes `Arc::new(parking_lot::Mutex<OffsetDateTime>)` to
    /// advance time without wall-clock sleeps.
    pub clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    /// Stable instance identifier embedded in authorization codes and access
    /// tokens as the `iss` audience component.
    pub instance_id: uuid::Uuid,
    /// Whether Dynamic Client Registration (RFC 7591 / DCR) is enabled.
    pub dcr_enabled: bool,
    /// Whether Client-Initiated Management Delete (RFC 7592) is enabled.
    pub cimd_enabled: bool,
}

impl OAuthState {
    /// Returns a disabled placeholder [`OAuthState`].
    ///
    /// Used by [`crate::app_state::AppStateBuilder::build`] when no explicit
    /// `OAuthState` has been wired (e.g. existing deployments that have not
    /// configured `oauth.mcp_enabled = true`).
    ///
    /// All fields carry inert placeholder values.  Route handlers MUST check
    /// `state.oauth.enabled` before doing any OAuth-related work.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            canonical: disabled_placeholder(),
            signer: Arc::new(McpOAuthJwtSigner::new(b"disabled-placeholder-not-used")),
            verifier: Arc::new(McpOAuthJwtVerifier::new(
                b"disabled-placeholder-not-used",
                "https://disabled.invalid".into(),
                vec![],
            )),
            clock: Arc::new(OffsetDateTime::now_utc),
            instance_id: uuid::Uuid::nil(),
            dcr_enabled: false,
            cimd_enabled: false,
        }
    }
}
