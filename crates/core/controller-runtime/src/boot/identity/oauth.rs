//! Phase 7d: OAuth boot.
//!
//! Boots the MCP OAuth 2.1 authorization-server state when enabled, and
//! captures the `(instance_id, db_conn)` pair needed for graceful shutdown
//! cleanup.

use rootcause::prelude::*;
use uptrakit_web_api::oauth::OAuthState;
use uptrakit_web_api::oauth::boot::boot_oauth_state;

use crate::AppError;

/// Phase 7d: boot OAuth state and capture the shutdown handle.
///
/// Returns `(oauth_state, oauth_instance_for_shutdown)`.
/// `oauth_instance_for_shutdown` is `Some((instance_id, db))` when OAuth is
/// enabled, used by the graceful-shutdown path to call
/// `uptrakit_web_api::oauth::boot::deregister_oauth_instance`.
pub(super) async fn boot(
    db: &sea_orm::DatabaseConnection,
) -> crate::Result<(
    OAuthState,
    Option<(uuid::Uuid, sea_orm::DatabaseConnection)>,
)> {
    let oauth_state = boot_oauth_state(db)
        .await
        .context(AppError::Config("OAuth boot failed".into()))?;

    // Capture before oauth_state is consumed by the builder; used for graceful cleanup
    // on both the SIGTERM/SIGINT path and the reexec path.
    let oauth_instance_for_shutdown = oauth_state
        .enabled
        .then_some((oauth_state.instance_id, db.clone()));

    Ok((oauth_state, oauth_instance_for_shutdown))
}
